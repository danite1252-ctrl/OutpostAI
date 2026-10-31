// A 3D model from the gallery (.glb), shown so it can be turned around with the mouse:
// drag to orbit, scroll to zoom. three.js loads only when a model is opened.

import { useEffect, useRef, useState } from "react";

/** `src`: the model file, already fetched (an object URL). */
export function ModelView({ src, className = "" }: { src: string; className?: string }) {
  const host = useRef<HTMLDivElement>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let stop = () => {};
    let cancelled = false;
    void (async () => {
      try {
        const [THREE, { GLTFLoader }, { OrbitControls }, blob] = await Promise.all([
          import("three"),
          import("three/examples/jsm/loaders/GLTFLoader.js"),
          import("three/examples/jsm/controls/OrbitControls.js"),
          fetch(src).then((r) => r.blob()),
        ]);
        const el = host.current;
        if (cancelled || !el) return;
        const data = await blob.arrayBuffer();
        const gltf = await new Promise<{ scene: import("three").Group }>((resolve, reject) =>
          new GLTFLoader().parse(data, "", resolve, reject),
        );
        if (cancelled) return;

        const scene = new THREE.Scene();
        scene.background = new THREE.Color(0x1b181c);
        scene.add(new THREE.HemisphereLight(0xffffff, 0x444444, 2.2));
        const sun = new THREE.DirectionalLight(0xffffff, 2.4);
        sun.position.set(2, 3, 4);
        scene.add(sun);
        const back = new THREE.DirectionalLight(0xffffff, 0.8);
        back.position.set(-3, -1, -2);
        scene.add(back);

        // Centre the model and fit it in view.
        const model = gltf.scene;
        const box = new THREE.Box3().setFromObject(model);
        const size = box.getSize(new THREE.Vector3()).length() || 1;
        model.position.sub(box.getCenter(new THREE.Vector3()));
        // A grey shape (no texture) gets a soft clay look so its form shows.
        model.traverse((o) => {
          const mesh = o as import("three").Mesh;
          if (mesh.isMesh) {
            const mat = mesh.material as import("three").MeshStandardMaterial;
            if (mat && !mat.map && !mesh.geometry.getAttribute("color")) {
              mesh.material = new THREE.MeshStandardMaterial({ color: 0xc9c2bb, roughness: 0.8, metalness: 0.05 });
            }
          }
        });
        scene.add(model);

        const camera = new THREE.PerspectiveCamera(40, el.clientWidth / Math.max(1, el.clientHeight), size / 100, size * 100);
        camera.position.set(0, size * 0.25, size * 1.4);
        const renderer = new THREE.WebGLRenderer({ antialias: true });
        renderer.setPixelRatio(window.devicePixelRatio);
        renderer.setSize(el.clientWidth, el.clientHeight);
        renderer.outputColorSpace = THREE.SRGBColorSpace;
        el.appendChild(renderer.domElement);
        const controls = new OrbitControls(camera, renderer.domElement);
        controls.enableDamping = true;
        controls.autoRotate = true;
        controls.autoRotateSpeed = 1.2;
        controls.addEventListener("start", () => (controls.autoRotate = false));

        let frame = 0;
        const tick = () => {
          controls.update();
          renderer.render(scene, camera);
          frame = requestAnimationFrame(tick);
        };
        tick();
        const resize = new ResizeObserver(() => {
          camera.aspect = el.clientWidth / Math.max(1, el.clientHeight);
          camera.updateProjectionMatrix();
          renderer.setSize(el.clientWidth, el.clientHeight);
        });
        resize.observe(el);
        setLoading(false);
        stop = () => {
          cancelAnimationFrame(frame);
          resize.disconnect();
          controls.dispose();
          scene.traverse((o) => {
            const mesh = o as import("three").Mesh;
            if (mesh.isMesh) {
              mesh.geometry.dispose();
              const mats = Array.isArray(mesh.material) ? mesh.material : [mesh.material];
              mats.forEach((m) => m.dispose());
            }
          });
          renderer.dispose();
          renderer.domElement.remove();
        };
      } catch (e) {
        if (!cancelled) {
          setLoading(false);
          setError(e instanceof Error ? e.message : "This model couldn't be shown.");
        }
      }
    })();
    return () => {
      cancelled = true;
      stop();
    };
  }, [src]);

  return (
    <div className={`relative overflow-hidden rounded-xl bg-surface ${className}`}>
      <div ref={host} className="absolute inset-0" aria-label="3D model: drag to turn it, scroll to zoom" role="img" />
      {loading && !error && <p className="absolute inset-0 grid place-items-center text-sm text-muted">Loading the model…</p>}
      {error && <p className="absolute inset-0 grid place-items-center p-4 text-center text-sm text-danger">{error}</p>}
    </div>
  );
}

/** A cube for 3D models in the gallery grid (they have no picture of their own). */
export function ModelTile({ size = 40 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.5} strokeLinejoin="round" aria-hidden="true">
      <path d="M12 2.5l8.5 4.8v9.4L12 21.5l-8.5-4.8V7.3z M3.5 7.3L12 12l8.5-4.7 M12 12v9.5" />
    </svg>
  );
}
