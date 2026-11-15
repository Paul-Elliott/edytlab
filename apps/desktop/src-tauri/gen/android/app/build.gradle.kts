import java.util.Properties
import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("rust")
}

val tauriProperties = Properties().apply {
    val propFile = file("tauri.properties")
    if (propFile.exists()) {
        propFile.inputStream().use { load(it) }
    }
}

android {
    compileSdk = 37
    namespace = "app.edytlab.android"
    defaultConfig {
        manifestPlaceholders["usesCleartextTraffic"] = "false"
        applicationId = "app.edytlab.android"
        minSdk = 26
        targetSdk = 37
        versionCode = tauriProperties.getProperty("tauri.android.versionCode", "1").toInt()
        versionName = tauriProperties.getProperty("tauri.android.versionName", "1.0")
    }
    // One fixed key for every build of this fork, debug or release, local or
    // CI, so a newer APK installs over an older one and keeps the projects
    // on the device. It is committed on purpose and protects nothing: it is
    // for sideloading only and must never sign a Play Store upload.
    signingConfigs {
        create("sideload") {
            storeFile = file("sideload.keystore")
            storePassword = "edytlab-sideload"
            keyAlias = "sideload"
            keyPassword = "edytlab-sideload"
        }
    }
    // Extract native libraries to the app's library folder at install. The
    // ONNX Runtime loader opens libonnxruntime.so by its file path
    // (ml_pipeline::runtime), and a library left inside the APK has none.
    packaging {
        jniLibs {
            useLegacyPackaging = true
            // The ONNX Runtime package also carries the bridge its Java API
            // uses. The app calls the C API from Rust, so that library would
            // only be weight.
            excludes.add("**/libonnxruntime4j_jni.so")
        }
    }
    buildTypes {
        getByName("debug") {
            signingConfig = signingConfigs.getByName("sideload")
            manifestPlaceholders["usesCleartextTraffic"] = "true"
            isDebuggable = true
            isJniDebuggable = true
            isMinifyEnabled = false
            packaging {
                jniLibs.keepDebugSymbols.add("*/arm64-v8a/*.so")
                jniLibs.keepDebugSymbols.add("*/armeabi-v7a/*.so")
                jniLibs.keepDebugSymbols.add("*/x86/*.so")
                jniLibs.keepDebugSymbols.add("*/x86_64/*.so")
            }
        }
        getByName("release") {
            signingConfig = signingConfigs.getByName("sideload")
            optimization {
               enable = true
            }
            proguardFiles(
                *fileTree(".") {
                  include("**/*.pro")
                  exclude("build/**")
                }.files.toTypedArray()
            )
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_1_8
        targetCompatibility = JavaVersion.VERSION_1_8
    }
    buildFeatures {
        buildConfig = true
    }
}

kotlin {
    compilerOptions {
        jvmTarget = JvmTarget.JVM_1_8
    }
}

rust {
    rootDirRel = "../../../"
}

dependencies {
    implementation("androidx.webkit:webkit:1.14.0")
    implementation("androidx.appcompat:appcompat:1.7.1")
    implementation("androidx.activity:activity-ktx:1.10.1")
    implementation("com.google.android.material:material:1.12.0")
    implementation("androidx.lifecycle:lifecycle-process:2.10.0")
    // ONNX Runtime, for transcription, speaker labels and stem separation.
    // Only its native library is used (jni/<abi>/libonnxruntime.so), which
    // the Rust side loads by path. The version is the one
    // src-tauri/ml-runtime.json pins for every platform; a test in
    // crates/ml-pipeline holds the two equal.
    implementation("com.microsoft.onnxruntime:onnxruntime-android:1.23.2")
    testImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test.ext:junit:1.1.4")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.5.0")
}

apply(from = file("tauri.build.gradle.kts"))
