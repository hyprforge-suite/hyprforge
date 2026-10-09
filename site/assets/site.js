// Hyprforge website. Everything here is progressive: without JavaScript
// every page reads top to bottom and every picture is there.
(() => {
  const $ = (s, el = document) => el.querySelector(s);
  const $$ = (s, el = document) => [...el.querySelectorAll(s)];

  // ---- the menu on small screens ----
  const menuBtn = $(".menu-btn");
  if (menuBtn) menuBtn.addEventListener("click", () => {
    const open = $(".nav").classList.toggle("open");
    menuBtn.setAttribute("aria-expanded", open);
  });

  // ---- things settle into place once, as you reach them ----
  $$("[data-stagger]").forEach((group) => {
    [...group.children].forEach((c, i) => {
      c.classList.add("rise");
      c.style.transitionDelay = `${Math.min(i * 60, 360)}ms`;
    });
  });
  const io = new IntersectionObserver((entries) => {
    for (const e of entries) if (e.isIntersecting) { e.target.classList.add("in"); io.unobserve(e.target); }
  }, { rootMargin: "0px 0px -6% 0px", threshold: 0.06 });
  $$(".rise").forEach((el) => io.observe(el));

  // ---- copy buttons on code ----
  $$(".codeblock").forEach((block) => {
    const btn = document.createElement("button");
    btn.className = "copy"; btn.type = "button"; btn.textContent = "copy";
    btn.addEventListener("click", async () => {
      const text = $("pre", block).innerText.split("\n")
        .map((l) => l.replace(/^\$ /, "").replace(/\s+#.*$/, "")).filter((l) => l.trim() && !l.trim().startsWith("#")).join("\n");
      try { await navigator.clipboard.writeText(text); btn.textContent = "copied"; }
      catch { btn.textContent = "select it instead"; }
      setTimeout(() => (btn.textContent = "copy"), 1600);
    });
    block.appendChild(btn);
  });

  // ---- the live lock screen ----
  // A recreation of what hyprforge-lock does: an idle clock, and a frosted
  // card the moment you type. Nothing typed is kept — only how many
  // characters there were, which is all the card ever shows.
  const screen = $(".screen");
  if (screen) {
    const time = $(".clock .time", screen), date = $(".clock .date", screen);
    const tick = () => {
      const d = new Date();
      time.textContent = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hour12: false });
      date.textContent = d.toLocaleDateString(undefined, { weekday: "long", day: "numeric", month: "long" });
    };
    tick(); setInterval(tick, 1000);

    const field = $(".field", screen), caps = $(".caps", screen), input = $(".lock-input", screen);
    let count = 0, idleTimer;
    const render = () => {
      $$(".dot", field).forEach((d) => d.remove());
      const caret = $(".caret", field);
      for (let i = 0; i < Math.min(count, 14); i++) {
        const dot = document.createElement("i"); dot.className = "dot"; field.insertBefore(dot, caret);
      }
    };
    const wake = () => {
      screen.classList.add("typing");
      clearTimeout(idleTimer);
      idleTimer = setTimeout(() => { if (!count) screen.classList.remove("typing"); }, 6000);
    };
    const sleep = () => { count = 0; render(); screen.classList.remove("typing"); };
    const unlock = () => { screen.classList.add("unlocked"); screen.classList.remove("typing"); count = 0; render(); input.blur(); };
    const key = (e) => {
      const on = e.getModifierState && e.getModifierState("CapsLock");
      if (caps) { caps.classList.toggle("caps-on", !!on); caps.textContent = on ? "Caps Lock on" : "Caps Lock off"; }
      if (e.key === "Escape") { sleep(); return true; }
      if (e.key === "Enter") {
        if (!count) { field.classList.remove("bad"); void field.offsetWidth; field.classList.add("bad"); return true; }
        unlock(); return true;
      }
      if (e.key === "Backspace") { count = Math.max(0, count - 1); render(); wake(); return true; }
      if (e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey) { count++; render(); wake(); return true; }
      return false;
    };

    screen.addEventListener("click", (e) => {
      if (e.target.closest(".relock")) { screen.classList.remove("unlocked"); return; }
      if (screen.classList.contains("unlocked")) return;
      input.focus({ preventScroll: true }); wake();
    });
    input.addEventListener("keydown", (e) => { if (key(e)) e.preventDefault(); });
    input.addEventListener("input", () => { input.value = ""; });
    // Any key wakes the real one, so any key wakes this one too — while
    // it is on screen and you are not typing somewhere else.
    addEventListener("keydown", (e) => {
      if (e.target === input || /INPUT|TEXTAREA|SELECT/.test(document.activeElement?.tagName || "")) return;
      if (screen.classList.contains("unlocked") || e.ctrlKey || e.metaKey || e.altKey) return;
      if (e.key.length !== 1 && e.key !== "Enter" && e.key !== "Backspace") return;
      const r = screen.getBoundingClientRect();
      if (r.bottom < 80 || r.top > innerHeight - 80) return;
      input.focus({ preventScroll: true });
      if (key(e)) e.preventDefault();
    });
  }

  // ---- Files: a search that understands filters ----
  const search = $("#files-search");
  if (search) {
    const input = $("input", search), out = $("#files-results");
    const files = [
      ["main.rs", "rs", 14e3, 1, "src/"], ["browser.rs", "rs", 96e3, 3, "src/"], ["archive.rs", "rs", 41e3, 12, "src/"],
      ["holiday-2026.mp4", "mp4", 1.4e9, 40, "Videos/"], ["render.blend", "blend", 220e6, 2, "Projects/"],
      ["benchy.stl", "stl", 11e6, 5, "Models/"], ["notes.md", "md", 3e3, 0, "Documents/"], ["TODO.md", "md", 1e3, 1, "Projects/"],
      ["IMG_4410.jpg", "jpg", 6.2e6, 9, "Pictures/"], ["IMG_4411.jpg", "jpg", 5.8e6, 9, "Pictures/"], ["backup.tar.zst", "zst", 4.1e9, 30, "Backups/"],
      ["invoice-0921.pdf", "pdf", 220e3, 18, "Documents/"], ["linux-6.6.tar.xz", "xz", 140e6, 60, "Downloads/"],
    ];
    const human = (n) => n >= 1e9 ? (n / 1e9).toFixed(1) + " GB" : n >= 1e6 ? (n / 1e6).toFixed(1) + " MB" : Math.round(n / 1e3) + " kB";
    const unit = { k: 1e3, K: 1e3, M: 1e6, G: 1e9 };
    const chips = [];
    const parse = (tok) => {
      let m;
      if ((m = tok.match(/^ext:(\w+)$/))) return { label: "ext:", value: m[1], test: (f) => f[1] === m[1] };
      if ((m = tok.match(/^size:([<>])(\d+(?:\.\d+)?)([kKMG])?$/))) {
        const n = parseFloat(m[2]) * (unit[m[3]] || 1);
        return { label: "size:", value: m[1] + m[2] + (m[3] || ""), test: (f) => (m[1] === ">" ? f[2] > n : f[2] < n) };
      }
      if ((m = tok.match(/^modified:<(\d+)d$/))) return { label: "modified:", value: "<" + m[1] + "d", test: (f) => f[3] < +m[1] };
      return null;
    };
    const draw = () => {
      $$(".chip", search).forEach((c) => c.remove());
      chips.forEach((c, i) => {
        const el = document.createElement("span"); el.className = "chip";
        el.innerHTML = `<b>${c.label}</b>${c.value}<button type="button" aria-label="Remove this filter">×</button>`;
        $("button", el).onclick = () => { chips.splice(i, 1); draw(); input.focus(); };
        search.insertBefore(el, input);
      });
      const words = input.value.trim().toLowerCase();
      const hits = files.filter((f) => chips.every((c) => c.test(f)) && (!words || f[0].toLowerCase().includes(words)));
      out.innerHTML = hits.length
        ? hits.map((f, i) => `<div style="animation-delay:${i * 18}ms"><b>${f[0]}</b><span>${f[4]}</span><span>${human(f[2])}</span></div>`).join("")
        : `<span class="empty">Nothing matches every filter.</span>`;
    };
    const commit = () => {
      const keep = [];
      for (const p of input.value.split(/\s+/)) { const c = p && parse(p); if (c) chips.push(c); else if (p) keep.push(p); }
      input.value = keep.join(" ");
    };
    input.addEventListener("keydown", (e) => {
      if (e.key === " " || e.key === "Enter") { const n = chips.length; commit(); if (chips.length !== n) e.preventDefault(); draw(); }
      if (e.key === "Backspace" && !input.value && chips.length) { chips.pop(); draw(); }
    });
    input.addEventListener("input", draw);
    $$(".suggest button").forEach((b) => b.addEventListener("click", () => {
      input.value = (input.value + " " + b.dataset.q).trim(); commit(); draw(); input.focus();
    }));
    draw();
  }

  // ---- Settings: every page, browsable ----
  const sd = $(".settings-demo");
  if (sd) {
    const buttons = $$("nav button", sd), panes = $$(".pane > div", sd);
    buttons.forEach((b) => b.addEventListener("click", () => {
      buttons.forEach((x) => x.classList.toggle("on", x === b));
      panes.forEach((p) => (p.hidden = p.dataset.page !== b.dataset.page));
    }));
  }
})();
