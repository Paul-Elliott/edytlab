package app.edytlab.android

import android.os.Bundle
import android.system.Os
import androidx.activity.enableEdgeToEdge

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    // The Rust side asks the process environment where temporary files and
    // the home folder are (std::env::temp_dir, std::env::home_dir, the
    // tempfile crate). Android sets neither to a folder the app may write
    // to, so point both inside the app's own storage before any Rust code
    // runs: super.onCreate is what starts it.
    Os.setenv("TMPDIR", cacheDir.absolutePath, true)
    Os.setenv("HOME", applicationInfo.dataDir, true)
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
  }
}
