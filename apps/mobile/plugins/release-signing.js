// Signs release builds of the phone app with Outpost's own release key, when the
// release script provides it (OUTPOST_KEYSTORE and OUTPOST_KEYSTORE_PASSWORD).
// Without them (everyday test builds) release builds keep using the debug key.
// Our own config plugin (MIT); it edits android/app/build.gradle after `expo prebuild`.
const { withAppBuildGradle } = require("expo/config-plugins");

const MARK = "// outpost-release-signing";

module.exports = function withReleaseSigning(config) {
  return withAppBuildGradle(config, (cfg) => {
    let gradle = cfg.modResults.contents;
    if (gradle.includes(MARK)) return cfg;
    gradle = gradle.replace(
      /signingConfigs\s*\{/,
      `signingConfigs {
        ${MARK}
        release {
            if (System.getenv("OUTPOST_KEYSTORE")) {
                storeFile file(System.getenv("OUTPOST_KEYSTORE"))
                storePassword System.getenv("OUTPOST_KEYSTORE_PASSWORD")
                keyAlias System.getenv("OUTPOST_KEY_ALIAS") ?: "outpost"
                keyPassword System.getenv("OUTPOST_KEYSTORE_PASSWORD")
            }
        }`,
    );
    gradle = gradle.replace(
      /(buildTypes\s*\{[\s\S]*?release\s*\{[\s\S]*?)signingConfig\s+signingConfigs\.debug/,
      `$1signingConfig System.getenv("OUTPOST_KEYSTORE") ? signingConfigs.release : signingConfigs.debug`,
    );
    cfg.modResults.contents = gradle;
    return cfg;
  });
};
