# Fix Gradle Sync Error: AGP 8.x Incompatibility with Gradle 9.6+

The project is currently using Gradle 9.7.0, which removed internal APIs (`InternalProblems`) that Android Gradle Plugin (AGP) 8.11.1 relies on. This causes the sync error reported.

## Proposed Changes

### Build Configuration

#### [MODIFY] [gradle-wrapper.properties](file:///D:/kanari-sdk/packages/kanari-kotlin/android/gradle/wrapper/gradle-wrapper.properties)
Downgrade the Gradle version from 9.7.0 to 9.5.

## Verification Plan

### Automated Tests
- Run `gradle_sync` to ensure the project syncs successfully.
- Run `gradle_build` on the `:kanari-crypto` and `:sample` modules to verify the project still builds.
