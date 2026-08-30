import java.io.File

plugins {
    id("com.android.application")
    kotlin("android")
}

val repoRoot = rootProject.projectDir.parentFile
val cargoLock = File(repoRoot, "Cargo.lock")
val rustManifest = File(repoRoot, "rust/phonecam-mobile-core/Cargo.toml")
val rustSources = File(repoRoot, "rust")
val udlFile = File(repoRoot, "rust/phonecam-mobile-core/src/phonecam.udl")
val rustJniLibsDir = layout.buildDirectory.dir("rustJniLibs/android").get().asFile
val uniffiOutDir = layout.buildDirectory.dir("generated/source/uniffi")

android {
    namespace = "com.phonecam.app"
    compileSdk = 34
    ndkVersion = "26.1.10909125"

    defaultConfig {
        applicationId = "com.phonecam.app"
        minSdk = 26
        targetSdk = 34
        versionCode = 1
        versionName = "0.1.0"

        ndk {
            abiFilters += listOf("arm64-v8a", "armeabi-v7a", "x86_64")
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
    }

    sourceSets {
        getByName("main") {
            jniLibs.srcDir(rustJniLibsDir)
        }
    }
}

dependencies {
    implementation("androidx.core:core-ktx:1.13.1")
    implementation("androidx.appcompat:appcompat:1.7.0")
    implementation("com.google.android.material:material:1.12.0")

    val cameraXVersion = "1.3.4"
    implementation("androidx.camera:camera-core:$cameraXVersion")
    implementation("androidx.camera:camera-camera2:$cameraXVersion")
    implementation("androidx.camera:camera-lifecycle:$cameraXVersion")
    implementation("androidx.camera:camera-view:$cameraXVersion")
    implementation("com.google.mlkit:barcode-scanning:17.3.0")

    val lifecycleVersion = "2.8.6"
    implementation("androidx.lifecycle:lifecycle-runtime-ktx:$lifecycleVersion")
    implementation("androidx.lifecycle:lifecycle-process:$lifecycleVersion")

    // Needed by generated UniFFI Kotlin bindings and direct Rust C FFI calls via JNA.
    implementation("net.java.dev.jna:jna:5.14.0@aar")

    testImplementation("junit:junit:4.13.2")
    testImplementation("net.java.dev.jna:jna:5.14.0")
}

val ensureRustTargets by tasks.registering(Exec::class) {
    description = "Ensure Android Rust targets are installed"
    commandLine(
        "rustup",
        "target",
        "add",
        "aarch64-linux-android",
        "armv7-linux-androideabi",
        "x86_64-linux-android",
    )
}

val ensureCargoNdk by tasks.registering(Exec::class) {
    description = "Install cargo-ndk if missing"
    commandLine(
        "sh",
        "-c",
        "command -v cargo-ndk >/dev/null 2>&1 || cargo install cargo-ndk --version 4.1.2 --locked",
    )
}

val buildRustAndroid by tasks.registering(Exec::class) {
    description = "Build Rust Android shared libraries with cargo-ndk"
    dependsOn(ensureRustTargets, ensureCargoNdk)

    inputs.file(rustManifest)
    inputs.file(cargoLock)
    inputs.dir(rustSources)
    outputs.dir(rustJniLibsDir)

    workingDir = repoRoot
    commandLine(
        "cargo",
        "ndk",
        "-t",
        "arm64-v8a",
        "-t",
        "armeabi-v7a",
        "-t",
        "x86_64",
        "-o",
        rustJniLibsDir.absolutePath,
        "build",
        "--manifest-path",
        rustManifest.absolutePath,
        "--release",
        "--locked",
    )
}

val ensureUniffiBindgen by tasks.registering(Exec::class) {
    description = "Install uniffi-bindgen CLI if missing"
    commandLine(
        "sh",
        "-c",
        "command -v uniffi-bindgen >/dev/null 2>&1 || cargo install uniffi --version 0.31.2 --locked --features cli",
    )
}

val generateUniffiBindings by tasks.registering(Exec::class) {
    description = "Generate UniFFI Kotlin bindings"
    dependsOn(ensureUniffiBindgen)

    val outDir = uniffiOutDir.get().asFile
    inputs.file(udlFile)
    outputs.dir(outDir)

    commandLine(
        "uniffi-bindgen",
        "generate",
        udlFile.absolutePath,
        "--language",
        "kotlin",
        "--out-dir",
        outDir.absolutePath,
    )
}

val buildRustHostForTests by tasks.registering(Exec::class) {
    description = "Build host Rust cdylib for JVM unit tests"
    workingDir = repoRoot
    commandLine(
        "cargo",
        "build",
        "--manifest-path",
        rustManifest.absolutePath,
        "--locked",
    )
}

kotlin {
    sourceSets.getByName("main").kotlin.srcDir(uniffiOutDir)
}

tasks.named("preBuild") {
    dependsOn(buildRustAndroid, generateUniffiBindings)
}

tasks.withType<Test>().configureEach {
    dependsOn(buildRustHostForTests)
    val hostLibDir = File(repoRoot, "target/debug").absolutePath
    environment("JNA_LIBRARY_PATH", hostLibDir)
    systemProperty("jna.library.path", hostLibDir)
}
