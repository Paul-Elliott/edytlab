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

/**
 * What the Rust side of edytlab needs from Android and cannot reach itself:
 * the files behind the system picker's `content://` addresses, the share
 * sheet, the microphone permission, and a few facts about the device.
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
}
