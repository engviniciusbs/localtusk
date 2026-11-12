/* LocalTusk — site.
   - Sem rastreamento e sem requisições a terceiros, exceto a página de download, que consulta
     a API pública do GitHub para listar a versão mais recente.
   - Movimento só com transform/opacity. Com "reduzir movimento" ligado, nada anima. */
(() => {
  "use strict";

  const doc = document.documentElement;
  const $ = (sel, root = document) => root.querySelector(sel);
  const $$ = (sel, root = document) => [...root.querySelectorAll(sel)];
  const REPO = "jhonatanMP/localtusk";

  /* ------------------------------------------------------------ Cabeçalho */
  function initHeader() {
    const header = $(".site-header");
    if (!header) return;
    const onScroll = () => header.classList.toggle("scrolled", window.scrollY > 12);
    onScroll();
    window.addEventListener("scroll", onScroll, { passive: true });

    const btn = $(".menu-btn");
    const links = $("#nav-links");
    if (!btn || !links) return;
    const set = (open) => {
      links.classList.toggle("open", open);
      btn.setAttribute("aria-expanded", String(open));
      btn.setAttribute("aria-label", open ? "Fechar menu" : "Abrir menu");
    };
    btn.addEventListener("click", () => set(!links.classList.contains("open")));
    links.addEventListener("click", (e) => { if (e.target.closest("a")) set(false); });
    document.addEventListener("keydown", (e) => { if (e.key === "Escape") set(false); });
    document.addEventListener("click", (e) => { if (!header.contains(e.target)) set(false); });
  }

  /* ------------------------------------------------------------ Abas (galeria) */
  function initTabs() {
    $$("[data-tabs]").forEach((root) => {
      const tabs = $$('[role="tab"]', root);
      const panels = tabs.map((t) => document.getElementById(t.getAttribute("aria-controls")));
      const select = (i, focus) => {
        tabs.forEach((t, n) => {
          const on = n === i;
          t.setAttribute("aria-selected", String(on));
          t.tabIndex = on ? 0 : -1;
          panels[n].hidden = !on;
        });
        if (focus) tabs[i].focus();
        const shot = $(".shot img", panels[i]);
        if (window.gsap && !doc.classList.contains("no-motion") && shot) {
          gsap.fromTo(shot, { opacity: 0, y: 14 }, { opacity: 1, y: 0, duration: 0.45, ease: "power3.out", clearProps: "transform" });
        }
      };
      tabs.forEach((t, i) => {
        t.addEventListener("click", () => select(i));
        t.addEventListener("keydown", (e) => {
          const k = { ArrowRight: 1, ArrowLeft: -1 }[e.key];
          if (k) { e.preventDefault(); select((i + k + tabs.length) % tabs.length, true); }
          if (e.key === "Home") { e.preventDefault(); select(0, true); }
          if (e.key === "End") { e.preventDefault(); select(tabs.length - 1, true); }
        });
      });
    });
  }

  /* ------------------------------------------------------------ Copiar comandos */
  function initCopy() {
    $$("[data-copy]").forEach((btn) => {
      btn.addEventListener("click", async () => {
        const target = document.getElementById(btn.dataset.copy);
        if (!target) return;
        const text = target.innerText.replace(/^\s*#.*$/gm, "").replace(/\n{2,}/g, "\n").trim();
        const label = $("span", btn);
        try {
          await navigator.clipboard.writeText(text);
          label.textContent = "Copiado";
        } catch {
          label.textContent = "Selecione e copie";
        }
        setTimeout(() => (label.textContent = "Copiar"), 1800);
      });
    });
  }

  /* ------------------------------------------------------------ Download */
  const OS_RULES = [
    { os: "macos", test: (n) => /\.(dmg|app\.tar\.gz)$/i.test(n) },
    { os: "windows", test: (n) => /\.(msi|exe)$/i.test(n) },
    { os: "linux", test: (n) => /\.(appimage|deb|rpm)$/i.test(n) },
  ];
  const fmtSize = (b) => (b > 1e6 ? (b / 1e6).toFixed(1) + " MB" : Math.max(1, Math.round(b / 1e3)) + " KB");

  function detectOS() {
    const p = (navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || navigator.userAgent || "";
    if (/mac/i.test(p)) return "macos";
    if (/win/i.test(p)) return "windows";
    if (/linux|x11/i.test(p)) return "linux";
    return "";
  }

  async function initDownload() {
    const page = $("[data-download]");
    if (!page) return;
    const os = detectOS();
    $$(".os-card").forEach((c) => {
      if (c.dataset.os === os) {
        c.classList.add("rec");
        const tag = $(".you", c);
        if (tag) tag.hidden = false;
      }
    });

    const box = $("#release");
    if (!box) return;
    const title = $("[data-release-title]", box);
    const text = $("[data-release-text]", box);
    const list = $("[data-release-assets]", box);
    try {
      // Lista com 1 item: devolve [] (200) quando ainda não há versão publicada, em vez de um 404.
      const res = await fetch(`https://api.github.com/repos/${REPO}/releases?per_page=1`, { headers: { Accept: "application/vnd.github+json" } });
      if (!res.ok) throw new Error(String(res.status));
      const rel = (await res.json()).find((r) => !r.draft);
      if (!rel) return; // ainda sem versão publicada: mantém o texto padrão
      const assets = (rel.assets || []).filter((a) => !/\.(sig|sha256|txt|json)$/i.test(a.name));
      title.textContent = `Versão ${rel.tag_name || rel.name}`;
      const when = rel.published_at ? new Date(rel.published_at).toLocaleDateString("pt-BR", { day: "numeric", month: "long", year: "numeric" }) : "";
      text.textContent = assets.length
        ? `Publicada em ${when}. Escolha o arquivo do seu sistema.`
        : `Publicada em ${when}, mas ainda sem instaladores anexados. Veja a página da versão no GitHub.`;
      assets.forEach((a) => {
        const rule = OS_RULES.find((r) => r.test(a.name));
        const link = document.createElement("a");
        link.className = "asset";
        link.href = a.browser_download_url;
        link.rel = "noopener";
        const name = document.createElement("span");
        name.textContent = a.name;
        const meta = document.createElement("small");
        meta.textContent = `${rule ? { macos: "macOS", windows: "Windows", linux: "Linux" }[rule.os] + " · " : ""}${fmtSize(a.size)}`;
        link.append(name, meta);
        list.append(link);
        if (rule && rule.os === os) link.style.borderColor = "var(--accent)";
      });
      list.hidden = !assets.length;
    } catch {
      text.textContent = "Não foi possível consultar o GitHub agora. Veja as versões diretamente na página de releases.";
    }
  }

  /* ------------------------------------------------------------ Movimento (GSAP) */
  function splitWords(el) {
    const walk = (node) => {
      [...node.childNodes].forEach((child) => {
        if (child.nodeType === 3) {
          const frag = document.createDocumentFragment();
          child.textContent.split(/(\s+)/).forEach((part) => {
            if (!part) return;
            if (/^\s+$/.test(part)) { frag.append(" "); return; }
            const w = document.createElement("span");
            w.className = "word";
            const inner = document.createElement("span");
            inner.className = "word-inner";
            inner.textContent = part;
            w.append(inner);
            frag.append(w);
          });
          child.replaceWith(frag);
        } else if (child.nodeType === 1) walk(child);
      });
    };
    walk(el);
    return $$(".word-inner", el);
  }

  function initMotion() {
    if (!window.gsap || !window.ScrollTrigger) { doc.classList.add("no-motion"); return; }
    gsap.registerPlugin(ScrollTrigger);
    const mm = gsap.matchMedia();

    mm.add("(prefers-reduced-motion: reduce)", () => { doc.classList.add("no-motion"); });

    mm.add("(prefers-reduced-motion: no-preference)", () => {
      doc.classList.remove("no-motion");
      doc.classList.add("motion-ready");

      // Barra de progresso de leitura (só transform).
      gsap.to(".progress", { scaleX: 1, ease: "none", scrollTrigger: { start: 0, end: "max", scrub: 0.2 } });

      // Entrada do hero.
      const h1 = $(".hero h1");
      if (h1) {
        const words = splitWords(h1);
        gsap.set(h1, { opacity: 1 });
        gsap.set(".hero [data-hero]", { opacity: 1 });
        const tl = gsap.timeline({ defaults: { ease: "power3.out" } });
        tl.from(".hero .pill", { y: 14, opacity: 0, duration: 0.6 })
          .from(words, { yPercent: 115, duration: 0.9, stagger: 0.06, ease: "power4.out" }, "-=0.35")
          .from(".hero-sub", { y: 18, opacity: 0, duration: 0.7 }, "-=0.55")
          .from(".hero-cta > *", { y: 14, opacity: 0, duration: 0.55, stagger: 0.08 }, "-=0.45")
          .from(".hero-chips li", { y: 10, opacity: 0, duration: 0.45, stagger: 0.06 }, "-=0.35")
          .from(".stage .window", { y: 70, opacity: 0, rotationX: 14, scale: 0.94, duration: 1.2, ease: "power3.out" }, "-=0.7");
      }

      // Brilhos de fundo: deriva lenta, pausa quando o hero sai da tela.
      const glows = $$(".glow");
      if (glows.length) {
        const drift = gsap.timeline({ repeat: -1, yoyo: true, defaults: { ease: "sine.inOut", duration: 11 } });
        drift.to(".glow-a", { x: 70, y: 30 }, 0).to(".glow-b", { x: -80, y: 50 }, 0);
        ScrollTrigger.create({ trigger: ".hero", start: "top bottom", end: "bottom top", onToggle: (self) => (self.isActive ? drift.play() : drift.pause()) });
      }

      // Janela do app sobe devagar enquanto o hero sai.
      if ($(".stage .window")) {
        gsap.to(".stage .window", { yPercent: -5, ease: "none", scrollTrigger: { trigger: ".stage", start: "top 80%", end: "bottom top", scrub: 0.6 } });
      }

      // Revelar blocos ao rolar.
      if ($("[data-reveal]")) {
        gsap.set("[data-reveal]", { y: 26 });
        ScrollTrigger.batch("[data-reveal]", {
          start: "top 90%",
          once: true,
          onEnter: (batch) => gsap.to(batch, { opacity: 1, y: 0, duration: 0.8, stagger: 0.09, ease: "power3.out", overwrite: true, clearProps: "transform" }),
        });
      }

      // Passos de "como funciona": o número acende ao entrar.
      $$(".steps li").forEach((li) => {
        gsap.from(li, { x: -22, opacity: 0, duration: 0.7, ease: "power3.out", scrollTrigger: { trigger: li, start: "top 88%", once: true } });
      });

      // Fluxo: pacote percorre navegador → Caddy → PHP-FPM → MySQL.
      $$(".lane").forEach((lane, idx) => {
        const row = $(".lane-row", lane);
        const packet = $(".packet", lane);
        const pings = $$(".ping", lane);
        const track = $(".track", lane);
        if (!row || !packet || !track) return;
        const dist = () => track.getBoundingClientRect().width;
        const tl = gsap.timeline({ repeat: -1, repeatDelay: 0.9, delay: idx * 1.1, paused: true });
        tl.fromTo(packet, { x: 0, opacity: 0 }, { x: dist, opacity: 1, duration: 3.2, ease: "none", keyframes: { opacity: [0, 1, 1, 1, 0] } }, 0);
        pings.forEach((p, i) => tl.fromTo(p, { scale: 0.92, opacity: 0.9 }, { scale: 1.12, opacity: 0, duration: 0.7, ease: "power2.out" }, (3.2 / (pings.length - 1)) * i));
        ScrollTrigger.create({ trigger: lane, start: "top 95%", end: "bottom 5%", invalidateOnRefresh: true, onToggle: (self) => (self.isActive ? tl.play() : tl.pause()) });
      });

      // Cartões do bento sobem em cascata (além do reveal).
      $$(".bento .card").forEach((c) => c.addEventListener("pointerenter", () => gsap.to($(".icon-chip", c), { y: -3, duration: 0.25, ease: "power2.out" })));
      $$(".bento .card").forEach((c) => c.addEventListener("pointerleave", () => gsap.to($(".icon-chip", c), { y: 0, duration: 0.3, ease: "power2.out" })));

      return () => doc.classList.remove("motion-ready");
    });
  }

  /* ------------------------------------------------------------ Início */
  initHeader();
  initTabs();
  initCopy();
  initDownload();
  initMotion();

  // Plano B: se o GSAP não carregar, o conteúdo nunca fica invisível.
  window.addEventListener("load", () => {
    if (!window.gsap) doc.classList.add("no-motion");
  });
})();
