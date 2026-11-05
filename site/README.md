# Outpost website

The public site for outpost.hartspace.cc: an introduction to Outpost and its privacy policy.
It's plain static files (no build step, no cookies, nothing loaded from other sites), so any
static host works: Cloudflare Pages, GitHub Pages, or a web server.

- `index.html`: the introduction
- `privacy.html`: the privacy policy (the link to give Google or anyone who asks)
- `assets/`: styles, the hero animation, logo images and fonts (Geist and Montserrat, both
  under the SIL Open Font License, see `assets/fonts/`)

To publish on Cloudflare Pages: create a Pages project, upload this folder (or connect the
repo with `site` as the build output folder and no build command), then add the custom
domain `outpost.hartspace.cc`.
