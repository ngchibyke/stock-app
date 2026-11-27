import java.util.Properties

plugins {
    id("com.android.application")
    // The Flutter Gradle Plugin must be applied after the Android and Kotlin Gradle plugins.
    id("dev.flutter.flutter-gradle-plugin")
}

// The upload key for release builds. It comes from android/key.properties (kept out of git:
// storeFile, storePassword, keyAlias, keyPassword) or from the environment, which is how the
// release workflow supplies it (ANDROID_KEYSTORE_PATH, ANDROID_KEYSTORE_PASSWORD,
// ANDROID_KEY_ALIAS, ANDROID_KEY_PASSWORD). Without it a release build stops with a message:
// a release is never signed with the debug key.
val keyProperties = Properties().apply {
    val file = rootProject.file("key.properties")
    if (file.exists()) file.inputStream().use { load(it) }
}

fun keySetting(env: String, prop: String): String? =
    System.getenv(env)?.takeIf { it.isNotBlank() } ?: keyProperties.getProperty(prop)

val uploadStoreFile = keySetting("ANDROID_KEYSTORE_PATH", "storeFile")
val uploadStorePassword = keySetting("ANDROID_KEYSTORE_PASSWORD", "storePassword")
val uploadKeyAlias = keySetting("ANDROID_KEY_ALIAS", "keyAlias")
val uploadKeyPassword = keySetting("ANDROID_KEY_PASSWORD", "keyPassword")
val hasUploadKey = listOf(uploadStoreFile, uploadStorePassword, uploadKeyAlias, uploadKeyPassword)
    .all { !it.isNullOrBlank() }

gradle.taskGraph.whenReady {
    val releaseBuild = allTasks.any {
        it.name == "assembleRelease" || it.name == "bundleRelease" || it.name == "packageRelease"
    }
    if (releaseBuild && !hasUploadKey) {
        throw GradleException(
            "A release build needs the upload key. Put storeFile, storePassword, keyAlias and " +
                "keyPassword in android/key.properties, or set ANDROID_KEYSTORE_PATH, " +
                "ANDROID_KEYSTORE_PASSWORD, ANDROID_KEY_ALIAS and ANDROID_KEY_PASSWORD. " +
                "Release builds are never signed with the debug key.",
        )
    }
}

android {
    namespace = "ng.market.market"
    compileSdk = flutter.compileSdkVersion
    ndkVersion = flutter.ndkVersion

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    defaultConfig {
        // TODO: Specify your own unique Application ID (https://developer.android.com/studio/build/application-id.html).
        applicationId = "ng.market.market"
        // You can update the following values to match your application needs.
        // For more information, see: https://flutter.dev/to/review-gradle-config.
        minSdk = 26
        targetSdk = flutter.targetSdkVersion
        // Uses the version code from pubspec.yaml. When using split APKs, 1000 * ABI_VERSION
        // is added automatically by Flutter. (https://developer.android.com/studio/build/configure-apk-splits#configure-APK-versions)
        // You can force using the value of versionCode by specifying the `-P force-version-code-ignoring-abi=true`
        // flag during build.
        versionCode = flutter.versionCode
        versionName = flutter.versionName
    }

    signingConfigs {
        create("release") {
            if (hasUploadKey) {
                storeFile = file(uploadStoreFile!!)
                storePassword = uploadStorePassword
                keyAlias = uploadKeyAlias
                keyPassword = uploadKeyPassword
            }
        }
    }

    buildTypes {
        release {
            signingConfig = signingConfigs.getByName("release")
        }
    }
}

kotlin {
    compilerOptions {
        jvmTarget = org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17
    }
}

flutter {
    source = "../.."
}
