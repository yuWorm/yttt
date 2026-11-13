import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.plugin.compose")
}

// Built by scripts/mobile/build-android.sh.
val coreOut = rootProject.layout.projectDirectory.dir("../../target/mobile/android")

android {
    namespace = "dev.yttt.spike"
    compileSdk = 37

    defaultConfig {
        applicationId = "dev.yttt.spike"
        minSdk = 26
        targetSdk = 35
        versionCode = 1
        versionName = "0.1-spike"
        ndk { abiFilters += "arm64-v8a" }
    }

    buildTypes {
        // Unsigned: signing needs the release key, kept out of the repo.
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures { compose = true }

    sourceSets.getByName("main") {
        kotlin.directories.add(coreOut.dir("kotlin").asFile.absolutePath)
        jniLibs.directories.add(coreOut.dir("jniLibs").asFile.absolutePath)
    }
}

kotlin {
    compilerOptions { jvmTarget.set(JvmTarget.JVM_17) }
}

dependencies {
    implementation("org.connectbot:termlib:0.3.13")
    // The file editor. LGPL-2.1: used as a separate, unmodified library.
    implementation("io.github.Rosemoe.sora-editor:editor:0.23.6")
    // Pairing QR codes: ZXing decodes the camera's frames (no Google services).
    implementation("com.google.zxing:core:3.5.4")
    implementation("androidx.camera:camera-camera2:1.6.2")
    implementation("androidx.camera:camera-lifecycle:1.6.2")
    implementation("androidx.camera:camera-view:1.6.2")
    implementation("net.java.dev.jna:jna:5.17.0@aar")
    implementation("androidx.activity:activity-compose:1.13.0")
    implementation("androidx.core:core-ktx:1.19.1")
    implementation(platform("androidx.compose:compose-bom:2026.09.00"))
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.material3:material3")
    testImplementation("junit:junit:4.13.2")
    // The real org.json: the android.jar one only throws in JVM tests.
    testImplementation("org.json:json:20250517")
}

// Release's own unit tests check that it reads no launch hooks; AGP only runs
// the debug build's by default.
androidComponents {
    beforeVariants(selector().withBuildType("release")) { variant ->
        variant.hostTests[com.android.build.api.variant.HostTestBuilder.UNIT_TEST_TYPE]?.enable = true
    }
}
