plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "com.kanari.sample"
    compileSdk = 37

    defaultConfig {
        applicationId = "com.kanari.sample"
        minSdk = 24
        //noinspection OldTargetApi,EditedTargetSdkVersion
        targetSdk = 37
        versionCode = 1
        versionName = "0.2.6"
    }

    buildTypes {
        getByName("debug") {
            // อนุญาตให้แอปใช้หน่วยความจำขนาดใหญ่
            // หมายเหตุ: ต้องเพิ่ม android:largeHeap="true" ใน AndroidManifest.xml ด้วย
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        compose = true
    }
}

kotlin {
    compilerOptions {
        jvmTarget = org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17
    }
}

dependencies {
    implementation(project(":kanari-crypto"))

    implementation(platform("androidx.compose:compose-bom:2026.08.00"))

    implementation("androidx.core:core-ktx:1.19.0")
    implementation("androidx.activity:activity-compose:1.13.0")
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.foundation:foundation")
    implementation("androidx.compose.foundation:foundation-layout:1.12.0")
    implementation("androidx.compose.material3:material3")
}
