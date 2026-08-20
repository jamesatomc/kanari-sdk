# Implementation Plan - Publish Kanari Crypto Library

Add Maven Publishing support to the `kanari-crypto` library to allow uploading it to repositories like JitPack or Maven Central.

## Proposed Changes

### [kanari-crypto](file:///D:/kanari-sdk/packages/kanari-kotlin/android/kanari-crypto)

Configure the `maven-publish` plugin to create an AAR artifact with the necessary metadata.

#### [MODIFY] [build.gradle.kts](file:///D:/kanari-sdk/packages/kanari-kotlin/android/kanari-crypto/build.gradle.kts)

- Add `maven-publish` plugin.
- Configure `publishing` block:
  - Define `groupId`: `com.kanari`
  - Define `artifactId`: `kanari-crypto`
  - Define `version`: `0.2.6` (matching the sample app version)
- Configure the component to be published (e.g., `release` AAR).

## Verification Plan

### Automated Tests

- Run `./gradlew :kanari-crypto:publishToMavenLocal` to verify that the library can be published to the local Maven repository.
- Inspect the generated POM file in `~/.m2/repository/com/kanari/kanari-crypto/0.2.6/` to ensure dependencies and metadata are correct.

### Manual Verification

- Verify that the native `.so` files are included in the generated AAR.
- Check if JitPack can build the project by creating a test tag/branch (if the user provides GitHub access).
