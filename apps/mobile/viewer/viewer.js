// The phone's 3D viewer, run inside a WebView (see src/screens/Gallery.tsx): three.js shows
// the model the app downloaded (handed over in pieces, see receive), turning slowly until
// it's touched. One finger turns it, two zoom. Bundled into src/viewer-html.ts by
// scripts/build-viewer.mjs when the app's packages are installed.

import * as THREE from "three";
import { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";

function say(type, message) {
  if (window.ReactNativeWebView) window.ReactNativeWebView.postMessage(JSON.stringify({ type, message }));
}

// The app hands the model over in base64 pieces (WebViews can't reliably read the phone's
// files): OUTPOST_PART for each, then OUTPOST_DONE.
function receive() {
  const parts = [];
  let total = 0;
  return new Promise((resolve) => {
    window.OUTPOST_PART = (b64) => {
      const bin = atob(b64);
      const bytes = new Uint8Array(bin.length);
      for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
      parts.push(bytes);
      total += bytes.length;
      say("part");
    };
    window.OUTPOST_DONE = () => {
      const all = new Uint8Array(total);
      let at = 0;
      for (const p of parts) {
        all.set(p, at);
        at += p.length;
      }
      parts.length = 0;
      resolve(all.buffer);
    };
    say("hello");
  });
}

async function main() {
  const data = await receive();
  const gltf = await new Promise((resolve, reject) => new GLTFLoader().parse(data, "", resolve, reject));

  const scene = new THREE.Scene();
  scene.background = new THREE.Color(window.OUTPOST_BG || 0x1b181c);
  scene.add(new THREE.HemisphereLight(0xffffff, 0x444444, 2.2));
  const sun = new THREE.DirectionalLight(0xffffff, 2.4);
  sun.position.set(2, 3, 4);
  scene.add(sun);
  const back = new THREE.DirectionalLight(0xffffff, 0.8);
  back.position.set(-3, -1, -2);
  scene.add(back);

  const model = gltf.scene;
  const box = new THREE.Box3().setFromObject(model);
  const size = box.getSize(new THREE.Vector3()).length() || 1;
  model.position.sub(box.getCenter(new THREE.Vector3()));
  // A plain shape gets a soft clay look so its form shows.
  model.traverse((o) => {
    if (o.isMesh && o.material && !o.material.map && !o.geometry.getAttribute("color")) {
      o.material = new THREE.MeshStandardMaterial({ color: 0xc9c2bb, roughness: 0.8, metalness: 0.05 });
    }
  });
  scene.add(model);

  const w = window.innerWidth;
  const h = window.innerHeight;
  const camera = new THREE.PerspectiveCamera(40, w / Math.max(1, h), size / 100, size * 100);
  camera.position.set(0, size * 0.25, size * 1.4);
  const renderer = new THREE.WebGLRenderer({ antialias: true });
  renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
  renderer.setSize(w, h);
  renderer.outputColorSpace = THREE.SRGBColorSpace;
  document.body.appendChild(renderer.domElement);
  const controls = new OrbitControls(camera, renderer.domElement);
  controls.enableDamping = true;
  controls.enablePan = false;
  controls.autoRotate = true;
  controls.autoRotateSpeed = 1.2;
  controls.addEventListener("start", () => (controls.autoRotate = false));
  window.addEventListener("resize", () => {
    camera.aspect = window.innerWidth / Math.max(1, window.innerHeight);
    camera.updateProjectionMatrix();
    renderer.setSize(window.innerWidth, window.innerHeight);
  });
  const tick = () => {
    controls.update();
    renderer.render(scene, camera);
    requestAnimationFrame(tick);
  };
  tick();
  say("ready");
}

main().catch((e) => say("error", e && e.message ? e.message : "This model couldn't be shown."));
