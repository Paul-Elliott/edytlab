package app.edytlab.android

import android.Manifest
import android.app.Activity
import android.app.ActivityManager
import android.content.ClipData
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.OpenableColumns
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import android.view.WindowManager
import android.webkit.MimeTypeMap
import androidx.core.content.FileProvider
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.Permission
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.File
import java.io.FileInputStream
import java.io.FileOutputStream
import java.io.IOException
import java.security.GeneralSecurityException
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import org.json.JSONObject

/** The Android Keystore, where the key that encrypts the saved secrets is. */
private const val ANDROID_KEYSTORE = "AndroidKeyStore"

/**
 * The name of that key. The `v1` is for a change of scheme: a new scheme
 * takes a new name, so it never reads a file the old one wrote.
 */
private const val SECRETS_KEY_ALIAS = "edytlab.secrets.v1"

private const val SECRETS_CIPHER = "AES/GCM/NoPadding"

/** The nonce the Keystore makes for each encryption, written before the ciphertext. */
private const val GCM_IV_BYTES = 12

private const val GCM_TAG_BITS = 128

@InvokeArg
class CopyUriToFileArgs {
  lateinit var uri: String
  lateinit var dir: String
}

@InvokeArg
class CopyFileToUriArgs {
  lateinit var path: String
  lateinit var uri: String
}

@InvokeArg
class ShareFileArgs {
  lateinit var path: String
  var mime: String? = null
  var title: String? = null
}

@InvokeArg
class KeepScreenOnArgs {
  var on: Boolean = false
}

@InvokeArg
class SecretArgs {
  lateinit var account: String
}

@InvokeArg
class SecretSetArgs {
  lateinit var account: String

  /** The bytes to keep, as base64. */
  lateinit var value: String
}

/**
 * What the Rust side of edytlab needs from Android and cannot reach itself:
 * the files behind the system picker's `content://` addresses, the share
 * sheet, the microphone permission, a few facts about the device, and the
 * Android Keystore, which is where the API keys are kept.
 *
 * Every command here is called from Rust only (`src/android.rs`). None is
 * granted to the webview, so the page gains no new way to read or write a
 * file: it passes an address the picker returned to a Rust command, and that
 * command decides where the bytes go.
 */
@TauriPlugin(
  permissions = [
    Permission(strings = [Manifest.permission.RECORD_AUDIO], alias = "microphone")
  ]
)
class EdytlabPlugin(private val activity: Activity) : Plugin(activity) {

  /**
   * Run `work` off the thread the command arrived on (the main thread, where
   * copying a large file would freeze the window) and answer the call with
   * what it returns, or with its error.
   */
  private fun inBackground(invoke: Invoke, work: () -> JSObject) {
    Thread {
      try {
        invoke.resolve(work())
      } catch (e: Throwable) {
        invoke.reject(e.message ?: e.toString())
      }
    }.start()
  }

  /** The name the picker shows for `uri`, if its provider says. */
  private fun displayName(uri: Uri): String? {
    try {
      activity.contentResolver.query(
        uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null
      )?.use { cursor ->
        if (cursor.moveToFirst()) {
          val index = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
          if (index >= 0) return cursor.getString(index)
        }
      }
    } catch (_: Exception) {
      // A provider that will not answer the query still lets the file be read.
    }
    return uri.lastPathSegment?.substringAfterLast('/')
  }

  /** A file name that is safe to create: no folders, no control characters. */
  private fun safeName(name: String): String {
    val cleaned = name
      .replace(Regex("[\\\\/:*?\"<>|\\p{Cntrl}]"), "_")
      .trim()
      .trimStart('.')
    return if (cleaned.isEmpty()) "audio" else cleaned.take(120)
  }

  /** `name` in `dir`, or `name (2)`, `name (3)`... if that is taken. */
  private fun freeFile(dir: File, name: String): File {
    var candidate = File(dir, name)
    if (!candidate.exists()) return candidate
    val dot = name.lastIndexOf('.')
    val stem = if (dot > 0) name.substring(0, dot) else name
    val ext = if (dot > 0) name.substring(dot) else ""
    var n = 2
    while (candidate.exists()) {
      candidate = File(dir, "$stem ($n)$ext")
      n += 1
    }
    return candidate
  }

  /**
   * Copy the file the picker named into `dir`, under the name the picker
   * showed. Answers `{ path, name }`.
   *
   * The copy is written beside its final name and renamed into place, so a
   * copy that is interrupted leaves no half file under a real name.
   */
  @Command
  fun copyUriToFile(invoke: Invoke) {
    val args = invoke.parseArgs(CopyUriToFileArgs::class.java)
    inBackground(invoke) {
      val uri = Uri.parse(args.uri)
      var name = safeName(displayName(uri) ?: "audio")
      if (!name.contains('.')) {
        // The decoder goes by the bytes, but a name with its extension is
        // what the track list shows.
        val mime = activity.contentResolver.getType(uri)
        val ext = mime?.let { MimeTypeMap.getSingleton().getExtensionFromMimeType(it) }
        if (!ext.isNullOrEmpty()) name = "$name.$ext"
      }
      val dir = File(args.dir)
      if (!dir.isDirectory && !dir.mkdirs()) {
        throw IOException("could not create ${dir.name}")
      }
      val dest = freeFile(dir, name)
      val part = File(dir, ".${dest.name}.part")
      try {
        val input = activity.contentResolver.openInputStream(uri)
          ?: throw IOException("the file could not be opened")
        input.use { source ->
          FileOutputStream(part).use { sink -> source.copyTo(sink, 1 shl 16) }
        }
        if (!part.renameTo(dest)) throw IOException("could not save ${dest.name}")
      } finally {
        part.delete()
      }
      val ret = JSObject()
      ret.put("path", dest.absolutePath)
      ret.put("name", dest.name)
      ret
    }
  }

  /**
   * Write the app's own file at `path` into the document the save dialog
   * created at `uri`. Answers `{ bytes }`.
   */
  @Command
  fun copyFileToUri(invoke: Invoke) {
    val args = invoke.parseArgs(CopyFileToUriArgs::class.java)
    inBackground(invoke) {
      val source = File(args.path)
      if (!source.isFile) throw IOException("${source.name} is not there to save")
      // "wt": replace what is there. A provider that reuses a document
      // would otherwise keep the tail of a longer file it held before.
      val output = activity.contentResolver.openOutputStream(Uri.parse(args.uri), "wt")
        ?: throw IOException("the chosen place could not be written to")
      var copied = 0L
      output.use { sink ->
        FileInputStream(source).use { input -> copied = input.copyTo(sink, 1 shl 16) }
      }
      val ret = JSObject()
      ret.put("bytes", copied)
      ret
    }
  }

  /**
   * Hand the file at `path` to the system share sheet.
   *
   * The receiving app reads it through the FileProvider the manifest
   * declares, which serves the cache folder: the file is copied to
   * `cache/share/` first, and what an earlier share left there is removed.
   */
  @Command
  fun shareFile(invoke: Invoke) {
    val args = invoke.parseArgs(ShareFileArgs::class.java)
    inBackground(invoke) {
      val source = File(args.path)
      if (!source.isFile) throw IOException("${source.name} is not there to share")
      val shareDir = File(activity.cacheDir, "share")
      shareDir.deleteRecursively()
      if (!shareDir.mkdirs()) throw IOException("could not prepare the file to share")
      val copy = File(shareDir, source.name)
      source.copyTo(copy, overwrite = true)
      val uri = FileProvider.getUriForFile(
        activity, "${activity.packageName}.fileprovider", copy
      )
      val mime = args.mime
        ?: MimeTypeMap.getSingleton().getMimeTypeFromExtension(source.extension.lowercase())
        ?: "application/octet-stream"
      val send = Intent(Intent.ACTION_SEND).apply {
        type = mime
        putExtra(Intent.EXTRA_STREAM, uri)
        clipData = ClipData.newRawUri(source.name, uri)
        addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
      }
      val chooser = Intent.createChooser(send, args.title ?: source.name)
      activity.runOnUiThread { activity.startActivity(chooser) }
      JSObject()
    }
  }

  /**
   * Keep the screen on, or let it sleep again. Android stops an app's work
   * soon after its screen goes off, which would end a long render, a model
   * download or a stem separation half way.
   */
  @Command
  fun keepScreenOn(invoke: Invoke) {
    val args = invoke.parseArgs(KeepScreenOnArgs::class.java)
    activity.runOnUiThread {
      if (args.on) {
        activity.window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
      } else {
        activity.window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
      }
    }
    invoke.resolve(JSObject())
  }

  /**
   * Facts about this device: its memory (which decides whether stem
   * separation may run), where the system unpacked the app's native
   * libraries (where the ONNX Runtime library is), and what it is.
   */
  @Command
  fun deviceInfo(invoke: Invoke) {
    val memory = ActivityManager.MemoryInfo()
    (activity.getSystemService(Context.ACTIVITY_SERVICE) as ActivityManager).getMemoryInfo(memory)
    val ret = JSObject()
    ret.put("totalRamBytes", memory.totalMem)
    ret.put("availableRamBytes", memory.availMem)
    ret.put("nativeLibraryDir", activity.applicationInfo.nativeLibraryDir)
    ret.put("sdkInt", Build.VERSION.SDK_INT)
    ret.put("model", "${Build.MANUFACTURER} ${Build.MODEL}")
    invoke.resolve(ret)
  }

  /**
   * Held across each secret command. Two saves at once would otherwise share
   * one temporary file, and two first saves would each make a Keystore key.
   */
  private val secretsLock = Any()

  /**
   * The folder the saved secrets are files in, one file per account.
   *
   * In `noBackupFilesDir`, beside `filesDir`: Android leaves this folder out
   * of every backup and of the copy it makes when a person moves to a new
   * phone. The Keystore key never leaves the device, so a file that did
   * arrive on another one could not be decrypted there, and a secret that
   * cannot be read is an error, not a missing one.
   */
  private fun secretsDir(): File = File(activity.noBackupFilesDir, "secrets")

  /**
   * The file for `account`. Letters, digits, `_` and `-` are kept and every
   * other byte is written as `%` and two hex digits, so no name can leave
   * the folder and two accounts never share a file.
   */
  private fun secretFile(account: String): File {
    val name = StringBuilder()
    for (byte in account.toByteArray(Charsets.UTF_8)) {
      val code = byte.toInt() and 0xff
      val c = code.toChar()
      if (c in 'a'..'z' || c in 'A'..'Z' || c in '0'..'9' || c == '_' || c == '-') {
        name.append(c)
      } else {
        name.append('%').append("0123456789abcdef"[code shr 4]).append("0123456789abcdef"[code and 0x0f])
      }
    }
    if (name.isEmpty() || name.length > 200) {
      throw IOException("that is not a name a secret can be saved under")
    }
    return File(secretsDir(), name.toString())
  }

  /**
   * The AES-256-GCM key in the Android Keystore that every secret is
   * encrypted with. It cannot be exported, and using it asks nothing of the
   * person: the app reads a key at launch, with no one there to unlock it.
   * `null` when there is none and `create` is false.
   */
  private fun secretsKey(create: Boolean): SecretKey? {
    val keyStore = KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }
    (keyStore.getKey(SECRETS_KEY_ALIAS, null) as? SecretKey)?.let { return it }
    if (!create) return null
    val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, ANDROID_KEYSTORE)
    generator.init(
      KeyGenParameterSpec.Builder(
        SECRETS_KEY_ALIAS,
        KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT
      )
        .setKeySize(256)
        .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
        .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
        .setUserAuthenticationRequired(false)
        .build()
    )
    return generator.generateKey()
  }

  /**
   * A Keystore failure as the error the Rust side is given: what failed and
   * the kind of exception, which says whether the key was invalidated or the
   * file did not match it. Never the exception's own text.
   */
  private fun keystoreFailure(what: String, e: Exception): IOException =
    IOException("$what (${e.javaClass.simpleName})")

  /**
   * `plain` encrypted for `account`: the nonce the Keystore chose, then the
   * ciphertext and its tag. The account's name is authenticated with it, so
   * one account's file is not accepted as another's.
   */
  private fun sealSecret(account: String, plain: ByteArray): ByteArray {
    try {
      val cipher = Cipher.getInstance(SECRETS_CIPHER)
      val key = secretsKey(create = true)
        ?: throw IOException("the Android Keystore made no key to encrypt the secret with")
      cipher.init(Cipher.ENCRYPT_MODE, key)
      cipher.updateAAD(account.toByteArray(Charsets.UTF_8))
      val sealed = cipher.doFinal(plain)
      val iv = cipher.iv
      if (iv == null || iv.size != GCM_IV_BYTES) {
        throw IOException("the Android Keystore did not encrypt the secret as expected")
      }
      return iv + sealed
    } catch (e: GeneralSecurityException) {
      throw keystoreFailure("the Android Keystore could not encrypt the secret", e)
    } catch (e: RuntimeException) {
      throw keystoreFailure("the Android Keystore could not encrypt the secret", e)
    }
  }

  /**
   * What [sealSecret] wrote, decrypted. Anything that stops it (a file cut
   * short, a key that is gone or was invalidated, a tag that does not match)
   * is an error. It is never answered as "nothing saved": that would tell the
   * app the person has no key, and the next save would write over this one.
   */
  private fun openSecret(account: String, stored: ByteArray): ByteArray {
    if (stored.size < GCM_IV_BYTES + GCM_TAG_BITS / 8) {
      throw IOException("the saved secret is damaged")
    }
    try {
      val key = secretsKey(create = false)
        ?: throw IOException("the key that protects the saved secrets is no longer in the Android Keystore")
      val cipher = Cipher.getInstance(SECRETS_CIPHER)
      cipher.init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(GCM_TAG_BITS, stored, 0, GCM_IV_BYTES))
      cipher.updateAAD(account.toByteArray(Charsets.UTF_8))
      return cipher.doFinal(stored, GCM_IV_BYTES, stored.size - GCM_IV_BYTES)
    } catch (e: GeneralSecurityException) {
      throw keystoreFailure("the saved secret could not be decrypted", e)
    } catch (e: RuntimeException) {
      throw keystoreFailure("the saved secret could not be decrypted", e)
    }
  }

  /**
   * Read the secret saved under `account`. Answers `{ value }`: the bytes as
   * base64, or `null` when nothing is saved there. A secret that is saved and
   * cannot be read is an error (see [openSecret]).
   */
  @Command
  fun secretGet(invoke: Invoke) {
    val args = invoke.parseArgs(SecretArgs::class.java)
    inBackground(invoke) {
      synchronized(secretsLock) {
        val file = secretFile(args.account)
        val ret = JSObject()
        if (file.exists()) {
          val plain = openSecret(args.account, file.readBytes())
          ret.put("value", Base64.encodeToString(plain, Base64.NO_WRAP))
        } else {
          // `put(name, null)` would leave the name out, and the Rust side
          // takes an answer with no `value` for a broken one.
          ret.put("value", JSONObject.NULL)
        }
        ret
      }
    }
  }

  /**
   * Save `value` (bytes, as base64) under `account`, replacing what was
   * there. The bytes are encrypted with the Keystore key and written beside
   * the file's final name, flushed, then renamed over it, so a save that is
   * interrupted leaves the old secret or the new one and never half of one.
   */
  @Command
  fun secretSet(invoke: Invoke) {
    val args = try {
      invoke.parseArgs(SecretSetArgs::class.java)
    } catch (_: Exception) {
      // Not the parser's own words: they can quote the text it was given,
      // and that text holds the secret.
      invoke.reject("the secret to save was not understood")
      return
    }
    inBackground(invoke) {
      val plain = try {
        Base64.decode(args.value, Base64.DEFAULT)
      } catch (_: IllegalArgumentException) {
        throw IOException("the secret to save did not arrive whole")
      }
      synchronized(secretsLock) {
        val dest = secretFile(args.account)
        val dir = secretsDir()
        if (!dir.isDirectory && !dir.mkdirs()) {
          throw IOException("could not create the folder for saved secrets")
        }
        val sealed = sealSecret(args.account, plain)
        val part = File(dir, ".${dest.name}.part")
        try {
          FileOutputStream(part).use { sink ->
            sink.write(sealed)
            sink.fd.sync()
          }
          if (!part.renameTo(dest)) throw IOException("could not save the secret")
        } finally {
          part.delete()
        }
        JSObject()
      }
    }
  }

  /**
   * Remove the secret saved under `account`. Nothing saved there is success.
   * The Keystore key stays: other accounts' files are encrypted with it.
   */
  @Command
  fun secretDelete(invoke: Invoke) {
    val args = invoke.parseArgs(SecretArgs::class.java)
    inBackground(invoke) {
      synchronized(secretsLock) {
        val file = secretFile(args.account)
        if (file.exists() && !file.delete()) {
          throw IOException("could not remove the saved secret")
        }
        JSObject()
      }
    }
  }
}
