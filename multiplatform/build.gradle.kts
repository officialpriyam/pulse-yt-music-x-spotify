plugins {
    kotlin("multiplatform") version "2.3.21"
}

kotlin {
    jvm("desktop")
    iosArm64()
    iosSimulatorArm64()

    sourceSets {
        commonMain {
            // Compile the existing Android sources directly so their behavior stays shared.
            kotlin.srcDir("../app/src/main/java")
            kotlin.include(
                "dev/priyx/pulse/data/util/SpotifyLinkParser.kt",
                "dev/priyx/pulse/backend/lyrics/TrackTitle.kt",
                "dev/priyx/pulse/backend/youtube/Thumbnails.kt",
            )
        }
        commonTest {
            dependencies {
                implementation(kotlin("test"))
            }
        }
    }
}
