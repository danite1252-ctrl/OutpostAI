// The hero picture: the household's devices around the outpost. Links draw in once, then
// messages travel home to the tower and replies travel back. Paused off-screen; still for
// anyone who prefers reduced motion.
(function () {
  const host = document.querySelector(".household");
  if (!host) return;
  const svg = host.querySelector("svg");
  const NS = "http://www.w3.org/2000/svg";
  const C = 280; // centre of the 560 × 560 picture
  const R = 222; // where the devices sit
  const EDGE = 104; // where a link meets the tower

  // 24 × 24 line icons.
  const GLYPHS = {
    phone: "M8 3h8a1 1 0 0 1 1 1v16a1 1 0 0 1-1 1H8a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1zM11 18h2",
    laptop: "M5 6h14v9H5zM3 18h18",
    desktop: "M3 5h18v11H3zM9 20h6M12 16v4",
    tablet: "M5 3h14a1 1 0 0 1 1 1v16a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1zM11 18h2",
    card: "M3 7h18v10H3zM7 17v3M17 17v3M7 10h4v4H7zM14 11h4M14 13h3",
  };
  const DEVICES = [
    { label: "Your phone", glyph: "phone", angle: -150 },
    { label: "Laptop", glyph: "laptop", angle: -90 },
    { label: "Their phone", glyph: "phone", angle: -30 },
    { label: "Gaming PC", glyph: "card", angle: 30 },
    { label: "Tablet", glyph: "tablet", angle: 90 },
    { label: "Family PC", glyph: "desktop", angle: 150 },
  ];

  const el = (name, attrs, parent) => {
    const e = document.createElementNS(NS, name);
    for (const k in attrs) e.setAttribute(k, attrs[k]);
    if (parent) parent.appendChild(e);
    return e;
  };
  const at = (deg, r) => {
    const a = (deg * Math.PI) / 180;
    return [C + r * Math.cos(a), C + r * Math.sin(a)];
  };

  const meshLayer = el("g", { "aria-hidden": "true" }, svg);
  const linkLayer = el("g", { "aria-hidden": "true" }, svg);
  const pulseLayer = el("g", { "aria-hidden": "true" }, svg);
  const nodeLayer = el("g", {}, svg);

  // A faint web between neighbours, like the network in the logo.
  DEVICES.forEach((d, i) => {
    const next = DEVICES[(i + 1) % DEVICES.length];
    const [x1, y1] = at(d.angle, R);
    const [x2, y2] = at(next.angle, R);
    const [mx, my] = at((d.angle + next.angle + (next.angle < d.angle ? 360 : 0)) / 2, R * 1.12);
    el("path", { class: "mesh", d: `M${x1} ${y1} Q${mx} ${my} ${x2} ${y2}` }, meshLayer);
  });

  const links = DEVICES.map((d, i) => {
    const [x1, y1] = at(d.angle, R - 30);
    const [x2, y2] = at(d.angle, EDGE);
    // A slight curve, alternating sides.
    const [cx, cy] = at(d.angle + (i % 2 ? 14 : -14), (R + EDGE) / 2);
    const path = el("path", { class: "link", d: `M${x1} ${y1} Q${cx} ${cy} ${x2} ${y2}` }, linkLayer);
    const len = path.getTotalLength();
    path.style.setProperty("--len", len);
    path.style.setProperty("--d", `${0.35 + i * 0.12}s`);

    const g = el("g", { class: "node", role: "img", "aria-label": d.label }, nodeLayer);
    g.style.setProperty("--d", `${0.2 + i * 0.12}s`);
    const [nx, ny] = at(d.angle, R);
    const ring = el("circle", { class: "ring", cx: nx, cy: ny, r: 30 }, g);
    el("path", { class: "glyph", d: GLYPHS[d.glyph], transform: `translate(${nx - 12} ${ny - 12})` }, g);
    const below = ny > C;
    el("text", { x: nx, y: below ? ny + 50 : ny - 42 }, g).textContent = d.label;
    return { path, len, ring };
  });

  host.classList.add("live");

  if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;

  // Messages home and replies back.
  const pulses = [];
  function send(i, outward) {
    const dot = el("circle", { class: "pulse", r: outward ? 3 : 3.6 }, pulseLayer);
    if (outward) dot.style.fill = "#ff6a3d";
    pulses.push({ dot, link: links[i], i, outward, t: 0, speed: 0.55 + Math.random() * 0.25 });
  }

  let visible = true;
  new IntersectionObserver((e) => (visible = e[0].isIntersecting)).observe(host);

  let last = performance.now();
  let nextSend = last + 2200; // after the links have drawn
  function frame(now) {
    const dt = Math.min(0.05, (now - last) / 1000);
    last = now;
    if (visible) {
      if (now > nextSend) {
        send(Math.floor(Math.random() * links.length), false);
        nextSend = now + 600 + Math.random() * 900;
      }
      for (let k = pulses.length - 1; k >= 0; k--) {
        const p = pulses[k];
        p.t += dt * p.speed;
        const along = p.outward ? 1 - p.t : p.t;
        const pt = p.link.path.getPointAtLength(Math.min(1, Math.max(0, along)) * p.link.len);
        p.dot.setAttribute("cx", pt.x);
        p.dot.setAttribute("cy", pt.y);
        p.dot.style.opacity = String(Math.min(1, Math.sin(Math.min(1, p.t) * Math.PI) * 1.6));
        if (p.t >= 1) {
          p.dot.remove();
          pulses.splice(k, 1);
          if (!p.outward) {
            // The outpost answers.
            setTimeout(() => send(p.i, true), 250 + Math.random() * 350);
          } else {
            p.link.ring.animate(
              [{ strokeWidth: 1.5 }, { strokeWidth: 4, stroke: "#ffb547" }, { strokeWidth: 1.5 }],
              { duration: 600, easing: "ease-out" },
            );
          }
        }
      }
    }
    requestAnimationFrame(frame);
  }
  requestAnimationFrame(frame);
})();

// Privacy page: mark the section being read in the contents.
(function () {
  const toc = document.querySelectorAll(".toc a");
  if (!toc.length) return;
  const byId = new Map([...toc].map((a) => [a.getAttribute("href").slice(1), a]));
  const seen = new IntersectionObserver(
    (entries) => {
      for (const e of entries) {
        if (!e.isIntersecting) continue;
        toc.forEach((a) => a.classList.remove("here"));
        byId.get(e.target.id)?.classList.add("here");
      }
    },
    { rootMargin: "-20% 0px -70% 0px" },
  );
  byId.forEach((_, id) => {
    const h = document.getElementById(id);
    if (h) seen.observe(h);
  });
})();

// Embers drifting up behind the hero.
(function () {
  const canvas = document.querySelector(".embers");
  if (!canvas || window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
  const ctx = canvas.getContext("2d");
  let w = 0, h = 0, ratio = 1;
  const sparks = [];
  function size() {
    ratio = Math.min(2, window.devicePixelRatio || 1);
    w = canvas.clientWidth;
    h = canvas.clientHeight;
    canvas.width = w * ratio;
    canvas.height = h * ratio;
    ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
  }
  function spark(anywhere) {
    return {
      x: w * (0.35 + Math.random() * 0.65),
      y: anywhere ? Math.random() * h : h + 10,
      r: 0.6 + Math.random() * 1.8,
      vy: 10 + Math.random() * 26,
      sway: Math.random() * Math.PI * 2,
      life: 0.4 + Math.random() * 0.6,
    };
  }
  size();
  window.addEventListener("resize", size);
  const count = Math.round(Math.min(46, w / 28));
  for (let i = 0; i < count; i++) sparks.push(spark(true));
  let visible = true;
  new IntersectionObserver((e) => (visible = e[0].isIntersecting)).observe(canvas);
  let last = performance.now();
  function frame(now) {
    const dt = Math.min(0.05, (now - last) / 1000);
    last = now;
    if (visible) {
      ctx.clearRect(0, 0, w, h);
      for (const s of sparks) {
        s.y -= s.vy * dt;
        s.sway += dt * 1.2;
        const x = s.x + Math.sin(s.sway) * 8;
        const fade = Math.max(0, Math.min(1, s.y / h)) * s.life * (0.75 + 0.25 * Math.sin(now / 180 + s.sway * 3));
        const g = ctx.createRadialGradient(x, s.y, 0, x, s.y, s.r * 4);
        g.addColorStop(0, `rgba(255, 196, 120, ${fade})`);
        g.addColorStop(0.4, `rgba(255, 106, 61, ${fade * 0.5})`);
        g.addColorStop(1, "rgba(255, 106, 61, 0)");
        ctx.fillStyle = g;
        ctx.beginPath();
        ctx.arc(x, s.y, s.r * 4, 0, Math.PI * 2);
        ctx.fill();
        if (s.y < -10) Object.assign(s, spark(false));
      }
    }
    requestAnimationFrame(frame);
  }
  requestAnimationFrame(frame);
})();

// "See it at work": an app window playing through a few scenes, with a phone beside it.
(function () {
  const show = document.querySelector(".showcase");
  if (!show) return;
  const still = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  const tabs = [...show.querySelectorAll('[role="tab"]')];
  const panels = tabs.map((t) => document.getElementById(t.getAttribute("aria-controls")));
  const where = show.querySelector(".bar .where");
  let run = 0;
  let current = 0;
  let visible = false;

  // Waits that end early (rejecting) when another scene starts.
  const sleep = (ms, id) =>
    new Promise((ok, stop) => {
      if (still) return id === run ? ok() : stop();
      setTimeout(() => (id === run ? ok() : stop()), ms);
    });
  const make = (tag, cls, parent, html) => {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    if (html != null) e.innerHTML = html;
    if (parent) parent.appendChild(e);
    return e;
  };
  async function type(field, text, id, speed = 32) {
    field.textContent = "";
    const caret = make("span", "caret");
    if (still) return void (field.textContent = text);
    field.appendChild(caret);
    for (const ch of text) {
      caret.before(ch);
      await sleep(speed + Math.random() * 30, id);
    }
    caret.remove();
  }
  async function stream(el, text, id, speed = 45) {
    if (still) return void (el.textContent = text);
    const words = text.split(" ");
    for (let i = 0; i < words.length; i++) {
      el.textContent += (i ? " " : "") + words[i];
      await sleep(speed + Math.random() * 40, id);
    }
  }
  async function send(panel, text, id) {
    const field = panel.querySelector(".composer .field");
    const convo = panel.querySelector(".convo");
    await type(field, text, id);
    await sleep(350, id);
    field.textContent = "";
    make("div", "msg me", convo).textContent = text;
    return convo;
  }
  async function thinking(convo, id) {
    const bubble = make("div", "msg it", convo, '<span class="typing"><i></i><i></i><i></i></span>');
    await sleep(900, id);
    bubble.innerHTML = "";
    return bubble;
  }

  const scenes = {
    async chat(panel, id) {
      const convo = await send(panel, "Plan a rainy Saturday for the kids, nothing too expensive", id);
      const reply = await thinking(convo, id);
      const lines = [
        "Here's a cosy one, about $30 all up:",
        "Morning: build a blanket fort, then a scavenger hunt with clues around the house.",
        "Lunch: make-your-own pizzas, everyone picks their toppings.",
        "Afternoon: the library for new books, then a movie in the fort.",
      ];
      for (const l of lines) {
        const p = make("p", "", reply);
        await stream(p, l, id);
        await sleep(200, id);
      }
    },

    async create(panel, id) {
      const field = panel.querySelector(".composer .field");
      const label = panel.querySelector(".status .label");
      const meter = panel.querySelector(".status .meter i");
      const status = panel.querySelector(".status");
      const canvas = panel.querySelector("canvas");
      const ctx = canvas.getContext("2d");
      const scene = sceneImage();
      status.style.opacity = "1";
      label.textContent = "Waiting for your idea";
      meter.style.width = "0";
      ctx.fillStyle = "#0c0a0d";
      ctx.fillRect(0, 0, 640, 400);
      await type(field, "a lighthouse on a cliff at sunset, oil painting", id);
      await sleep(300, id);
      label.textContent = "Making your picture";
      const noise = document.createElement("canvas");
      noise.width = 160;
      noise.height = 100;
      const nctx = noise.getContext("2d");
      const img = nctx.createImageData(160, 100);
      const start = performance.now();
      const length = still ? 0 : 4600;
      await new Promise((done, stop) => {
        function frame(now) {
          if (id !== run) return stop();
          const t = length ? Math.min(1, (now - start) / length) : 1;
          const p = 1 - Math.pow(1 - t, 2.2);
          ctx.filter = `blur(${(1 - p) * 16}px)`;
          ctx.drawImage(scene, 0, 0);
          ctx.filter = "none";
          if (p < 1) {
            for (let i = 0; i < img.data.length; i += 4) {
              const v = Math.random() * 255;
              img.data[i] = v;
              img.data[i + 1] = v * 0.7;
              img.data[i + 2] = v * 0.55;
              img.data[i + 3] = 255;
            }
            nctx.putImageData(img, 0, 0);
            ctx.globalAlpha = Math.pow(1 - p, 1.3);
            ctx.imageSmoothingEnabled = false;
            ctx.drawImage(noise, 0, 0, 640, 400);
            ctx.globalAlpha = 1;
          }
          meter.style.width = `${Math.round(t * 100)}%`;
          if (t < 1) requestAnimationFrame(frame);
          else done();
        }
        requestAnimationFrame(frame);
      });
      label.textContent = "Saved to your gallery";
      await sleep(1600, id);
      status.style.opacity = "0";
    },

    async voice(panel, id) {
      const box = panel.querySelector(".voice");
      const wave = panel.querySelector(".wave");
      const said = panel.querySelector(".said");
      const answer = panel.querySelector(".answer");
      if (!wave.children.length) for (let i = 0; i < 24; i++) make("i", "", wave);
      const bars = [...wave.children];
      said.textContent = "";
      answer.textContent = "";
      box.classList.add("listening");
      let talking = true;
      const level = () => {
        if (id !== run || !talking) return bars.forEach((b) => (b.style.height = "6px"));
        bars.forEach((b, i) => (b.style.height = `${6 + Math.abs(Math.sin(performance.now() / 160 + i)) * Math.random() * 46}px`));
        setTimeout(level, 110);
      };
      if (!still) level();
      await stream(said, "Turn the kitchen lamp on, and what's the weather tomorrow?", id, 170);
      talking = false;
      box.classList.remove("listening");
      await sleep(700, id);
      await stream(answer, "The kitchen lamp is on. Tomorrow looks sunny with a high of 24, so a good day to get outside.", id, 70);
    },

    async code(panel, id) {
      const convo = await send(panel, "Add a /spawn command to my plugin, then build it", id);
      await sleep(500, id);
      const acts = make("div", "actions-list", convo);
      for (const a of [
        "Looked in MyPlugin",
        "Read MyPlugin/src/main/java/plugin/Commands.java",
        'Changed MyPlugin/src/main/java/plugin/Commands.java <span class="ok">+18</span> −2',
        "Ran in MyPlugin: gradlew build",
      ]) {
        make("span", "", acts, a);
        await sleep(650, id);
      }
      const log = make("div", "log", convo);
      for (const l of ["&gt; Task :compileJava", "&gt; Task :processResources", "&gt; Task :jar", '<span class="ok">BUILD SUCCESSFUL in 41s</span>']) {
        make("div", "", log, l);
        await sleep(520, id);
      }
      const reply = make("div", "msg it", convo);
      await stream(reply, "Added /spawn with a permission check, and the build passed.", id);
      await sleep(300, id);
      make("div", "built", convo, "<span>Built MyPlugin/build/libs/MyPlugin-1.2.jar</span><b>Download</b>");
    },
  };

  // A small painted scene for Create: sky, sun, sea, a cliff and its lighthouse.
  let painted = null;
  function sceneImage() {
    if (painted) return painted;
    const c = document.createElement("canvas");
    c.width = 640;
    c.height = 400;
    const x = c.getContext("2d");
    const sky = x.createLinearGradient(0, 0, 0, 270);
    sky.addColorStop(0, "#1b1030");
    sky.addColorStop(0.55, "#a8344a");
    sky.addColorStop(0.85, "#ff6a3d");
    sky.addColorStop(1, "#ffb547");
    x.fillStyle = sky;
    x.fillRect(0, 0, 640, 270);
    const sun = x.createRadialGradient(430, 250, 10, 430, 250, 150);
    sun.addColorStop(0, "rgba(255,236,190,1)");
    sun.addColorStop(0.25, "rgba(255,200,120,0.9)");
    sun.addColorStop(1, "rgba(255,150,80,0)");
    x.fillStyle = sun;
    x.fillRect(0, 0, 640, 270);
    x.fillStyle = "#ffe7b8";
    x.beginPath();
    x.arc(430, 258, 38, 0, Math.PI * 2);
    x.fill();
    const sea = x.createLinearGradient(0, 265, 0, 400);
    sea.addColorStop(0, "#7a2e3a");
    sea.addColorStop(1, "#160d16");
    x.fillStyle = sea;
    x.fillRect(0, 265, 640, 135);
    for (let i = 0; i < 26; i++) {
      x.fillStyle = `rgba(255,${190 + Math.random() * 50},140,${0.5 - i / 60})`;
      const yy = 272 + i * 5;
      const ww = 80 - i * 2 + Math.random() * 30;
      x.fillRect(430 - ww / 2 + (Math.random() - 0.5) * 12, yy, ww, 2);
    }
    x.fillStyle = "#0d080d";
    x.beginPath();
    x.moveTo(0, 210);
    x.lineTo(70, 190);
    x.lineTo(150, 196);
    x.lineTo(205, 230);
    x.lineTo(232, 290);
    x.lineTo(260, 330);
    x.lineTo(300, 400);
    x.lineTo(0, 400);
    x.fill();
    // Lighthouse
    x.fillStyle = "#efe6dc";
    x.beginPath();
    x.moveTo(98, 194);
    x.lineTo(126, 194);
    x.lineTo(120, 104);
    x.lineTo(104, 104);
    x.fill();
    x.fillStyle = "#d8452a";
    for (const y of [124, 152, 178]) {
      x.beginPath();
      x.moveTo(104 - (y - 104) * 0.07, y);
      x.lineTo(120 + (y - 104) * 0.07, y);
      x.lineTo(121 + (y + 10 - 104) * 0.07, y + 10);
      x.lineTo(103 - (y + 10 - 104) * 0.07, y + 10);
      x.fill();
    }
    x.fillStyle = "#2a1a22";
    x.fillRect(100, 98, 24, 6);
    x.fillStyle = "#ffd77a";
    x.fillRect(105, 84, 14, 14);
    x.fillStyle = "#2a1a22";
    x.beginPath();
    x.moveTo(101, 84);
    x.lineTo(112, 72);
    x.lineTo(123, 84);
    x.fill();
    const beam = x.createLinearGradient(112, 90, 420, 60);
    beam.addColorStop(0, "rgba(255,225,150,0.55)");
    beam.addColorStop(1, "rgba(255,225,150,0)");
    x.fillStyle = beam;
    x.beginPath();
    x.moveTo(112, 90);
    x.lineTo(460, 30);
    x.lineTo(470, 110);
    x.fill();
    x.strokeStyle = "rgba(20,10,16,0.8)";
    x.lineWidth = 2;
    for (const [bx, by, s] of [[300, 120, 1], [330, 104, 0.8], [270, 140, 0.7]]) {
      x.beginPath();
      x.moveTo(bx - 9 * s, by);
      x.quadraticCurveTo(bx - 4 * s, by - 6 * s, bx, by);
      x.quadraticCurveTo(bx + 4 * s, by - 6 * s, bx + 9 * s, by);
      x.stroke();
    }
    painted = c;
    return c;
  }

  function reset(panel) {
    panel.querySelectorAll(".convo").forEach((c) => (c.innerHTML = ""));
    panel.querySelectorAll(".composer .field").forEach((f) => (f.textContent = ""));
  }

  let fillAnim = null;
  async function play(i, fromUser) {
    const id = ++run;
    current = i;
    tabs.forEach((t, k) => {
      const on = k === i;
      t.setAttribute("aria-selected", String(on));
      t.tabIndex = on ? 0 : -1;
      panels[k].hidden = !on;
      t.querySelector(".fill").style.width = "0";
    });
    where.textContent = tabs[i].textContent;
    fillAnim?.cancel();
    const panel = panels[i];
    reset(panel);
    const name = tabs[i].id.replace("tab-", "");
    const started = performance.now();
    try {
      await scenes[name](panel, id);
      if (still) return;
      // Show the finished scene for a moment, with the tab's bar filling, then the next one.
      const fill = tabs[i].querySelector(".fill");
      fill.style.width = "100%";
      fillAnim = fill.animate([{ width: "0%" }, { width: "100%" }], { duration: 3800, easing: "linear" });
      await sleep(3800, id);
      while (!visible) await sleep(500, id);
      play((i + 1) % tabs.length);
    } catch {
      // Another scene took over.
    }
    void started;
    void fromUser;
  }

  tabs.forEach((t, i) => {
    t.addEventListener("click", () => play(i, true));
    t.addEventListener("keydown", (e) => {
      const step = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
      if (!step) return;
      const next = (current + step + tabs.length) % tabs.length;
      tabs[next].focus();
      play(next, true);
    });
  });

  // The phone: a job finishes now and then; earlier ones stay below, dimmed.
  const stack = show.querySelector(".notes");
  const notes = [
    "Your video is ready: the lighthouse at sunset.",
    "Your 3D model is ready to print.",
    "Training finished: you can now draw Max by name.",
    "Your picture of the blanket fort is ready.",
  ];
  let n = 0;
  function push() {
    const el = make("div", "note", null, '<img src="assets/mark-64.png" alt=""><div><b>Outpost</b><span></span></div>');
    el.querySelector("span").textContent = notes[n++ % notes.length];
    stack.querySelectorAll(".note").forEach((o) => o.classList.add("old"));
    stack.prepend(el);
    while (stack.children.length > 3) stack.lastElementChild.remove();
    requestAnimationFrame(() => requestAnimationFrame(() => el.classList.add("in")));
  }
  async function notify() {
    if (!stack) return;
    for (;;) {
      await new Promise((r) => setTimeout(r, 1800));
      if (visible) {
        push();
        await new Promise((r) => setTimeout(r, 4400));
      }
    }
  }

  let begun = false;
  new IntersectionObserver(
    (e) => {
      visible = e[0].isIntersecting;
      if (visible && !begun) {
        begun = true;
        play(0);
        if (!still) notify();
        else if (stack) {
          push();
          push();
        }
      }
    },
    { threshold: 0.25 },
  ).observe(show.querySelector(".stage"));
})();

// Install guide: copy a command.
document.querySelectorAll(".code .copy").forEach((b) => {
  b.addEventListener("click", async () => {
    const text = b.parentElement.querySelector("code").textContent;
    try {
      await navigator.clipboard.writeText(text);
      b.textContent = "Copied";
      b.classList.add("done");
    } catch {
      b.textContent = "Select and copy";
    }
    setTimeout(() => {
      b.textContent = "Copy";
      b.classList.remove("done");
    }, 2000);
  });
});
