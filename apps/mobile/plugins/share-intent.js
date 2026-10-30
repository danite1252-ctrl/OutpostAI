// Puts Outpost in Android's share sheet (text, links, pictures, videos and documents), and
// lets Outpost links (pairing QR codes scanned with the camera) open the app.
// Our own config plugin (MIT); it edits the main activity in AndroidManifest.xml after
// `expo prebuild`.
const { withAndroidManifest } = require("expo/config-plugins");

// The name in the share sheet.
const LABEL = "Outpost";
const SEND_TYPES = [
  "text/*",
  "image/*",
  "video/*",
  "application/pdf",
  "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
];
const MULTIPLE_TYPES = ["image/*", "video/*", "application/pdf", "text/*"];

const filter = (action, types) => ({
  $: { "android:label": LABEL },
  action: [{ $: { "android:name": action } }],
  category: [{ $: { "android:name": "android.intent.category.DEFAULT" } }],
  data: types.map((t) => ({ $: { "android:mimeType": t } })),
});

module.exports = function withShareIntent(config) {
  return withAndroidManifest(config, (cfg) => {
    const app = cfg.modResults.manifest.application?.[0];
    const main = app?.activity?.find((a) => a.$["android:name"] === ".MainActivity");
    if (!main) throw new Error("share-intent: MainActivity not found");
    // Run again (a second prebuild): take out ours first.
    const ours = (f) =>
      (f.action ?? []).some((a) => /\.SEND(_MULTIPLE)?$/.test(a.$["android:name"])) ||
      (f.data ?? []).some((d) => d.$["android:scheme"] === "outpost");
    main["intent-filter"] = (main["intent-filter"] ?? []).filter((f) => !ours(f));
    // One running Outpost gets the share (no second copy on top).
    main.$["android:launchMode"] = "singleTask";
    main["intent-filter"].push(
      filter("android.intent.action.SEND", SEND_TYPES),
      filter("android.intent.action.SEND_MULTIPLE", MULTIPLE_TYPES),
      {
        action: [{ $: { "android:name": "android.intent.action.VIEW" } }],
        category: [
          { $: { "android:name": "android.intent.category.DEFAULT" } },
          { $: { "android:name": "android.intent.category.BROWSABLE" } },
        ],
        data: [{ $: { "android:scheme": "outpost", "android:host": "pair" } }],
      },
    );
    return cfg;
  });
};
