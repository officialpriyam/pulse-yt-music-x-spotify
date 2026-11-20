# Pulse shared Kotlin

This Gradle build compiles the Spotify link parser, track title and artist normalizers, and cover sizing functions directly from the existing Android sources. The Desktop JVM, iOS device, and iOS simulator targets use the same Kotlin files.

Run the desktop tests from the repository root.

```sh
./gradlew -p multiplatform desktopTest
```

Run `./gradlew -p multiplatform iosSimulatorArm64Test` on an Apple silicon Mac with Xcode and an iOS runtime.

## Port boundaries

| Part | Port approach |
| --- | --- |
| Pulse accounts, playlists and saved tracks | Use the existing Rust API and versioned migrations across clients. |
| Kotlin data and queue logic | Move platform-independent code into common sources as each feature is ported. |
| Screens and navigation | Port the Pulse design to Compose Multiplatform and use platform-specific navigation and window integrations where needed. |
| Playback and media controls | Keep a common player interface. Android uses Media3, AudioTrack and JNI. iOS needs an iOS audio and Now Playing implementation. Desktop needs an audio and media-key implementation for its OS. |
| OAuth and credential storage | Use the system browser and the target platform’s secure credential store. |
| Installing updates | Use the target platform’s distribution mechanism. The signed APK updater belongs to Android. |

This module establishes source reuse and test targets. The repository still needs native iOS and desktop app targets with UI and playback adapters.
