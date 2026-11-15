package app.edytlab.android

import android.graphics.Color
import android.os.Bundle
import android.system.Os
import android.view.View
import androidx.activity.SystemBarStyle
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    // The Rust side asks the process environment where temporary files and
    // the home folder are (std::env::temp_dir, std::env::home_dir, the
    // tempfile crate). Android sets neither to a folder the app may write
    // to, so point both inside the app's own storage before any Rust code
    // runs: super.onCreate is what starts it.
    Os.setenv("TMPDIR", cacheDir.absolutePath, true)
    Os.setenv("HOME", applicationInfo.dataDir, true)

    // The app is dark in both system themes, so the bar icons are light.
    enableEdgeToEdge(
      statusBarStyle = SystemBarStyle.dark(Color.TRANSPARENT),
      navigationBarStyle = SystemBarStyle.dark(Color.TRANSPARENT)
    )
    super.onCreate(savedInstanceState)

    // Android draws the app under the status bar, the navigation bar, a
    // display cutout and the on-screen keyboard. Keep the page out from
    // under all four by padding the view that holds the WebView, so the
    // page's own height is always the room that is really free: the header
    // clears the status bar, and the chat box rises with the keyboard. Done
    // here and not with CSS insets because how a WebView reports those
    // depends on its version, and the app runs on whichever one the device
    // has. The strips left behind show the window background
    // (res/values/themes.xml), which is the app's own.
    val content = findViewById<View>(android.R.id.content)
    ViewCompat.setOnApplyWindowInsetsListener(content) { view, insets ->
      val bars = insets.getInsets(
        WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout()
      )
      val keyboard = insets.getInsets(WindowInsetsCompat.Type.ime())
      view.setPadding(bars.left, bars.top, bars.right, maxOf(bars.bottom, keyboard.bottom))
      WindowInsetsCompat.CONSUMED
    }
  }
}
