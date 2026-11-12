import { open, save } from "@tauri-apps/plugin-dialog";
import { useEffect, useMemo, useRef, useState } from "react";
import { askConfirm } from "./confirm";
import { ApiResponse, CapturedRequest, HeaderPair, Project, api, formatBytes } from "./api";
import { Icon } from "./ui";

function errMsg(e: unknown) {
  return typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
}

const METHODS = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

// ------------------------------------------------------------------ utilidades

/** Se o texto for JSON, devolve formatado; senão, como está. */
function pretty(text: string): { text: string; json: boolean } {
  const t = text.trim();
  if (t.startsWith("{") || t.startsWith("[")) {
    try {
      return { text: JSON.stringify(JSON.parse(t), null, 2), json: true };
    } catch {
      /* não é JSON válido */
    }
  }
  return { text, json: false };
}

const shq = (s: string) => `'${s.replace(/'/g, `'\\''`)}'`;

/** Comando cURL equivalente à requisição. */
function toCurl(method: string, url: string, headers: HeaderPair[], body?: string | null) {
  const parts = ["curl", "-X", method, shq(url)];
  for (const [k, v] of headers) parts.push("-H", shq(`${k}: ${v}`));
  if (body) parts.push("--data-raw", shq(body));
  return parts.join(" ");
}

// Cabeçalhos que o cliente HTTP define sozinho; não entram no cURL nem no reenvio.
const AUTO_HEADERS = new Set(["host", "content-length", "connection", "accept-encoding", "transfer-encoding"]);
const userHeaders = (h: HeaderPair[]) => h.filter(([k]) => !AUTO_HEADERS.has(k.toLowerCase()));

function statusClass(status: number) {
  return status >= 500 ? "s5xx" : status >= 400 ? "s4xx" : status >= 300 ? "s3xx" : status >= 200 ? "s2xx" : "";
}

function clock(secs: number) {
  return new Date(secs * 1000).toLocaleTimeString("pt-BR");
}

function copy(text: string) {
  navigator.clipboard?.writeText(text).catch(() => {});
}

function Body({ text, truncated, empty }: { text: string | null; truncated?: boolean; empty: string }) {
  if (text === null) return <p className="muted small">Conteúdo binário (não exibido).</p>;
  if (text === "") return <p className="muted small">{empty}</p>;
  const p = pretty(text);
  return (
    <>
      <pre className={`code ${p.json ? "json" : ""}`}>{p.text}</pre>
      {truncated && <p className="hint">Mostrando só o começo do conteúdo (limite de 64 KB).</p>}
    </>
  );
}

function HeaderTable({ headers }: { headers: HeaderPair[] }) {
  if (headers.length === 0) return <p className="muted small">Nenhum cabeçalho.</p>;
  return (
    <dl className="kv headers">
      {headers.map(([k, v], i) => (
        <div className="kv-row" key={`${k}-${i}`}>
          <dt>{k}</dt>
          <dd>{v}</dd>
        </div>
      ))}
    </dl>
  );
}

// ------------------------------------------------------------------ estado do cliente

interface Row {
  id: number;
  enabled: boolean;
  key: string;
  value: string;
}

type BodyMode = "none" | "json" | "text" | "form";

interface ClientState {
  method: string;
  url: string;
  headers: Row[];
  bodyMode: BodyMode;
  body: string;
  followRedirects: boolean;
}

interface HistoryItem {
  state: ClientState;
  status: number | null;
  at: number;
}

let rowSeq = 1;
const row = (key = "", value = "", enabled = true): Row => ({ id: rowSeq++, enabled, key, value });

const initialClient = (p: Project): ClientState => ({
  method: "GET",
  url: `${p.url}/`,
  headers: [row("Accept", "application/json")],
  bodyMode: "none",
  body: "",
  followRedirects: false,
});

function load<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key);
    return raw ? (JSON.parse(raw) as T) : fallback;
  } catch {
    return fallback;
  }
}

/** Monta a requisição a enviar a partir do formulário. */
function buildRequest(p: Project, s: ClientState) {
  let url = s.url.trim();
  if (url.startsWith("/")) url = p.url + url; // "/api/users" vale para este projeto
  const headers: HeaderPair[] = s.headers.filter((r) => r.enabled && r.key.trim()).map((r) => [r.key.trim(), r.value]);
  const has = (name: string) => headers.some(([k]) => k.toLowerCase() === name);
  const body = s.bodyMode === "none" || s.body === "" ? null : s.body;
  if (body !== null && !has("content-type")) {
    const type = { json: "application/json", form: "application/x-www-form-urlencoded", text: "text/plain" }[
      s.bodyMode as Exclude<BodyMode, "none">
    ];
    headers.push(["Content-Type", type]);
  }
  return { method: s.method, url, headers, body, followRedirects: s.followRedirects };
}

/** Carrega no cliente uma requisição capturada pelo inspetor. */
function fromCaptured(p: Project, r: CapturedRequest): ClientState {
  const headers = userHeaders(r.requestHeaders).map(([k, v]) => row(k, v));
  const type = r.requestHeaders.find(([k]) => k.toLowerCase() === "content-type")?.[1] ?? "";
  const body = r.requestBody ?? "";
  const mode: BodyMode = !body ? "none" : type.includes("json") ? "json" : type.includes("x-www-form-urlencoded") ? "form" : "text";
  return { method: r.method, url: p.url + r.uri, headers, bodyMode: mode, body, followRedirects: false };
}

// ------------------------------------------------------------------ coleção

/** Requisição guardada na coleção do projeto (arquivo `collection.json`). */
interface SavedRequest {
  id: string;
  name: string;
  method: string;
  /** Relativa ao projeto (`/api/x`) quando aponta para ele; assim vale mesmo se a porta mudar. */
  url: string;
  headers: { key: string; value: string; enabled: boolean }[];
  bodyMode: BodyMode;
  body: string;
  followRedirects: boolean;
}

const BODY_MODES: BodyMode[] = ["none", "json", "text", "form"];
const newId = () => Math.random().toString(36).slice(2, 10) + Date.now().toString(36).slice(-4);

function toSaved(id: string, name: string, p: Project, s: ClientState): SavedRequest {
  return {
    id,
    name,
    method: s.method,
    url: s.url.trim().startsWith(p.url) ? s.url.trim().slice(p.url.length) || "/" : s.url.trim(),
    headers: s.headers.filter((r) => r.key.trim()).map((r) => ({ key: r.key, value: r.value, enabled: r.enabled })),
    bodyMode: s.bodyMode,
    body: s.body,
    followRedirects: s.followRedirects,
  };
}

function fromSaved(r: SavedRequest): ClientState {
  return {
    method: r.method,
    url: r.url,
    headers: r.headers.map((h) => row(h.key, h.value, h.enabled)),
    bodyMode: r.bodyMode,
    body: r.body,
    followRedirects: r.followRedirects,
  };
}

const serializeCollection = (items: SavedRequest[]) =>
  JSON.stringify({ format: "localtusk-collection", version: 1, requests: items }, null, 2);

/** Lê o texto de uma coleção, descartando entradas malformadas. Sempre gera ids novos se `fresh`. */
function parseCollection(text: string, fresh = false): SavedRequest[] {
  const doc = JSON.parse(text);
  const list: unknown[] = Array.isArray(doc?.requests) ? doc.requests : [];
  const out: SavedRequest[] = [];
  for (const raw of list) {
    const r = raw as Partial<SavedRequest> | null;
    if (!r || typeof r.method !== "string" || typeof r.url !== "string") continue;
    out.push({
      id: !fresh && typeof r.id === "string" && r.id ? r.id : newId(),
      name: typeof r.name === "string" && r.name.trim() ? r.name.trim() : `${r.method} ${r.url}`,
      method: METHODS.includes(r.method.toUpperCase()) ? r.method.toUpperCase() : "GET",
      url: r.url,
      headers: Array.isArray(r.headers)
        ? r.headers
            .filter((h) => h && typeof h.key === "string")
            .map((h) => ({ key: h.key, value: String(h.value ?? ""), enabled: h.enabled !== false }))
        : [],
      bodyMode: BODY_MODES.includes(r.bodyMode as BodyMode) ? (r.bodyMode as BodyMode) : "none",
      body: typeof r.body === "string" ? r.body : "",
      followRedirects: r.followRedirects === true,
    });
  }
  return out;
}

interface Collection {
  items: SavedRequest[];
  ready: boolean;
  activeId: string | null;
  name: string;
  note: string | null;
  setName: (n: string) => void;
  onSave: () => void;
  onUpdate: () => void;
  onOpen: (r: SavedRequest) => void;
  onDelete: (r: SavedRequest) => void;
  onExport: () => void;
  onImport: () => void;
}

// ------------------------------------------------------------------ componente principal

/** Ferramentas para criar APIs: inspetor das requisições recebidas e cliente para enviar. */
export default function ApiTools(props: { project: Project; onError: (m: string) => void }) {
  const { project: p } = props;
  const [view, setView] = useState<"inspector" | "client">("inspector");
  const [client, setClient] = useState<ClientState>(() => load(`localtusk.client.${p.id}`, initialClient(p)));
  const [history, setHistory] = useState<HistoryItem[]>(() => load(`localtusk.history.${p.id}`, []));
  const [response, setResponse] = useState<ApiResponse | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  const [saved, setSaved] = useState<SavedRequest[]>([]);
  const [ready, setReady] = useState(false);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [note, setNote] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    api
      .loadCollection(p.id)
      .then((text) => {
        if (!alive) return;
        if (text) setSaved(parseCollection(text));
        setReady(true);
      })
      .catch((e) => alive && props.onError(errMsg(e)));
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [p.id]);

  useEffect(() => {
    if (!note) return;
    const t = setTimeout(() => setNote(null), 5000);
    return () => clearTimeout(t);
  }, [note]);

  async function persist(next: SavedRequest[]): Promise<boolean> {
    const prev = saved;
    setSaved(next);
    try {
      await api.saveCollection(p.id, serializeCollection(next));
      return true;
    } catch (e) {
      setSaved(prev);
      props.onError(errMsg(e));
      return false;
    }
  }

  /** Guarda na coleção uma requisição vista no inspetor. */
  async function saveCaptured(r: CapturedRequest): Promise<boolean> {
    if (!ready) {
      props.onError("A coleção ainda está carregando. Tente de novo em instantes.");
      return false;
    }
    return persist([toSaved(newId(), `${r.method} ${r.uri}`, p, fromCaptured(p, r)), ...saved]);
  }

  const defaultName = () => {
    const path = client.url.trim().replace(p.url, "") || "/";
    return `${client.method} ${path}`;
  };

  const collection: Collection = {
    items: saved,
    ready,
    activeId,
    name,
    note,
    setName,
    onSave: async () => {
      const id = newId();
      const item = toSaved(id, name.trim() || defaultName(), p, client);
      if (await persist([item, ...saved])) {
        setActiveId(id);
        setName(item.name);
        setNote(`Salva em “${item.name}”.`);
      }
    },
    onUpdate: async () => {
      const current = saved.find((r) => r.id === activeId);
      if (!current) return;
      const item = toSaved(current.id, name.trim() || current.name, p, client);
      if (await persist(saved.map((r) => (r.id === item.id ? item : r)))) {
        setName(item.name);
        setNote(`“${item.name}” atualizada.`);
      }
    },
    onOpen: (r) => {
      setClient(fromSaved(r));
      setResponse(null);
      setFailure(null);
      setActiveId(r.id);
      setName(r.name);
    },
    onDelete: async (r) => {
      const ok = await askConfirm({
        title: "Remover da coleção?",
        message: `“${r.name}” será removida da coleção deste projeto.`,
        confirmLabel: "Remover",
        danger: true,
      });
      if (!ok) return;
      if (await persist(saved.filter((x) => x.id !== r.id)) && activeId === r.id) {
        setActiveId(null);
        setName("");
      }
    },
    onExport: async () => {
      try {
        const path = await save({
          defaultPath: `${p.id}-colecao.json`,
          filters: [{ name: "Coleção do LocalTusk", extensions: ["json"] }],
        });
        if (!path) return;
        await api.exportCollection(p.id, path);
        setNote(`Coleção exportada (${saved.length} ${saved.length === 1 ? "requisição" : "requisições"}).`);
      } catch (e) {
        props.onError(errMsg(e));
      }
    },
    onImport: async () => {
      try {
        const path = await open({
          multiple: false,
          directory: false,
          filters: [{ name: "Coleção do LocalTusk", extensions: ["json"] }],
        });
        if (typeof path !== "string") return;
        const incoming = parseCollection(await api.readCollectionFile(path), true);
        if (incoming.length === 0) {
          setNote("O arquivo não tem nenhuma requisição válida.");
          return;
        }
        if (await persist([...incoming, ...saved])) {
          setNote(`${incoming.length} ${incoming.length === 1 ? "requisição importada" : "requisições importadas"}.`);
        }
      } catch (e) {
        props.onError(errMsg(e));
      }
    },
  };

  useEffect(() => {
    localStorage.setItem(`localtusk.client.${p.id}`, JSON.stringify(client));
  }, [p.id, client]);
  useEffect(() => {
    localStorage.setItem(`localtusk.history.${p.id}`, JSON.stringify(history));
  }, [p.id, history]);

  async function send() {
    setSending(true);
    setFailure(null);
    const request = buildRequest(p, client);
    try {
      const r = await api.sendRequest(request);
      setResponse(r);
      remember(r.status);
    } catch (e) {
      setResponse(null);
      setFailure(errMsg(e));
      remember(null);
    } finally {
      setSending(false);
    }
  }

  function remember(status: number | null) {
    setHistory((h) => [{ state: client, status, at: Date.now() }, ...h].slice(0, 15));
  }

  function replay(r: CapturedRequest) {
    setClient(fromCaptured(p, r));
    setResponse(null);
    setFailure(null);
    setView("client");
  }

  return (
    <div className="api-tools">
      <nav className="subtabs" role="tablist">
        <button role="tab" aria-selected={view === "inspector"} className={view === "inspector" ? "active" : ""} onClick={() => setView("inspector")}>
          <Icon name="activity" size={14} /> Requisições recebidas
        </button>
        <button role="tab" aria-selected={view === "client"} className={view === "client" ? "active" : ""} onClick={() => setView("client")}>
          <Icon name="send" size={14} /> Cliente de API
        </button>
      </nav>
      {view === "inspector" ? (
        <Inspector project={p} onReplay={replay} onSave={saveCaptured} />
      ) : (
        <Client
          project={p}
          state={client}
          setState={setClient}
          response={response}
          failure={failure}
          sending={sending}
          onSend={send}
          history={history}
          onLoad={(s) => {
            setClient(s);
            setResponse(null);
            setFailure(null);
          }}
          onClearHistory={() => setHistory([])}
          collection={collection}
        />
      )}
    </div>
  );
}

// ------------------------------------------------------------------ inspetor

type Captured = CapturedRequest & { seq: number };

function Inspector(props: {
  project: Project;
  onReplay: (r: CapturedRequest) => void;
  onSave: (r: CapturedRequest) => Promise<boolean>;
}) {
  const { project: p } = props;
  const [items, setItems] = useState<Captured[]>([]);
  const [paused, setPaused] = useState(false);
  const [filter, setFilter] = useState("");
  const [selected, setSelected] = useState<number | null>(null);
  const [part, setPart] = useState<"request" | "response">("response");
  const offset = useRef(0);
  const seq = useRef(1);

  useEffect(() => {
    if (paused) return;
    let alive = true;
    const tick = async () => {
      try {
        const chunk = await api.readRequests(p.id, offset.current);
        if (!alive) return;
        offset.current = chunk.offset;
        if (chunk.entries.length > 0) {
          const added = chunk.entries.map((e) => ({ ...e, seq: seq.current++ }));
          setItems((prev) => [...prev, ...added].slice(-500));
        }
      } catch {
        /* tenta de novo no próximo ciclo */
      }
    };
    tick();
    const t = setInterval(tick, 1000);
    return () => {
      alive = false;
      clearInterval(t);
    };
  }, [p.id, paused]);

  const shown = useMemo(() => {
    const f = filter.trim().toLowerCase();
    const list = f
      ? items.filter((r) => `${r.method} ${r.uri} ${r.status}`.toLowerCase().includes(f))
      : items;
    return [...list].reverse();
  }, [items, filter]);
  // Sem escolha do usuário, mostra a requisição mais recente (e acompanha as novas).
  const effective = selected ?? shown[0]?.seq ?? null;
  const current = items.find((r) => r.seq === effective) ?? null;

  async function clear() {
    await api.clearRequests(p.id).catch(() => {});
    offset.current = 0;
    setItems([]);
    setSelected(null);
  }

  return (
    <div className="inspector">
      <div className="inspector-bar">
        <input
          className="grow"
          type="search"
          placeholder="Filtrar por método, caminho ou status…"
          aria-label="Filtrar requisições"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
        />
        <button className="btn small" onClick={() => setPaused((v) => !v)} aria-pressed={paused}>
          <Icon name={paused ? "play" : "stop"} size={13} /> {paused ? "Retomar" : "Pausar"}
        </button>
        <button className="btn small" onClick={clear} disabled={items.length === 0}>
          <Icon name="trash" size={13} /> Limpar
        </button>
      </div>

      {items.length === 0 ? (
        <div className="empty">
          <Icon name="activity" size={22} />
          <strong>Nenhuma requisição ainda</strong>
          <span className="muted small">
            {p.running ? (
              <>
                Chame o seu projeto em <code>{p.url}</code> (navegador, app, Postman…) ou use o Cliente de API: cada
                requisição que passar pelo PHP aparece aqui, com o que chegou e o que foi respondido.
              </>
            ) : (
              "Inicie o projeto para receber requisições."
            )}
          </span>
          <span className="muted small">Arquivos estáticos (imagens, CSS) não passam pelo PHP e não aparecem.</span>
        </div>
      ) : (
        <div className="inspector-body">
          <ul className="req-list" aria-label="Requisições recebidas">
            {shown.length === 0 && <li className="muted small pad">Nada combina com o filtro.</li>}
            {shown.map((r) => (
              <li key={r.seq}>
                <button className={`req-item ${r.seq === effective ? "active" : ""}`} aria-current={r.seq === effective} onClick={() => setSelected(r.seq)}>
                  <span className={`method m-${r.method.toLowerCase()}`}>{r.method}</span>
                  <span className="req-path ellipsis" title={r.uri}>
                    {r.uri}
                  </span>
                  <span className={`status ${statusClass(r.status)}`}>{r.status}</span>
                  <span className="muted small req-ms">{Math.round(r.durationMs)} ms</span>
                </button>
              </li>
            ))}
          </ul>

          <div className="req-detail">
            {!current ? (
              <p className="muted small pad">Selecione uma requisição para ver os detalhes.</p>
            ) : (
              <Detail
                key={current.seq}
                project={p}
                r={current}
                part={part}
                setPart={setPart}
                onReplay={() => props.onReplay(current)}
                onSave={() => props.onSave(current)}
              />
            )}
          </div>
        </div>
      )}
    </div>
  );
}

function Detail(props: {
  project: Project;
  r: Captured;
  part: "request" | "response";
  setPart: (p: "request" | "response") => void;
  onReplay: () => void;
  onSave: () => Promise<boolean>;
}) {
  const { r, project: p } = props;
  const [saved, setSaved] = useState(false);
  const query = useMemo(() => {
    const i = r.uri.indexOf("?");
    return i < 0 ? [] : [...new URLSearchParams(r.uri.slice(i + 1)).entries()];
  }, [r.uri]);
  const path = r.uri.split("?")[0];

  return (
    <div className="detail">
      <div className="detail-head">
        <div className="grow">
          <div className="detail-title">
            <span className={`method m-${r.method.toLowerCase()}`}>{r.method}</span>
            <code className="ellipsis" title={r.uri}>
              {path}
            </code>
          </div>
          <div className="muted small">
            <span className={`status ${statusClass(r.status)}`}>{r.status}</span> · {clock(r.time)} ·{" "}
            {Math.round(r.durationMs)} ms · resposta {formatBytes(r.responseSize)}
          </div>
        </div>
        <button
          className="btn small"
          onClick={() => copy(toCurl(r.method, p.url + r.uri, userHeaders(r.requestHeaders), r.requestBody))}
        >
          <Icon name="copy" size={13} /> cURL
        </button>
        <button
          className="btn small"
          disabled={saved}
          onClick={async () => setSaved(await props.onSave())}
          title="Guardar esta requisição na coleção do projeto"
        >
          <Icon name={saved ? "check" : "archive"} size={13} /> {saved ? "Salva" : "Salvar"}
        </button>
        <button className="btn small primary" onClick={props.onReplay}>
          <Icon name="send" size={13} /> Reenviar
        </button>
      </div>

      <div className="chips">
        <button className={`chip-btn ${props.part === "request" ? "active" : ""}`} onClick={() => props.setPart("request")}>
          Requisição
        </button>
        <button className={`chip-btn ${props.part === "response" ? "active" : ""}`} onClick={() => props.setPart("response")}>
          Resposta
        </button>
      </div>

      {props.part === "request" ? (
        <>
          {query.length > 0 && (
            <>
              <h4>Parâmetros da URL</h4>
              <dl className="kv headers">
                {query.map(([k, v], i) => (
                  <div className="kv-row" key={`${k}-${i}`}>
                    <dt>{k}</dt>
                    <dd>{v}</dd>
                  </div>
                ))}
              </dl>
            </>
          )}
          <h4>Cabeçalhos</h4>
          <HeaderTable headers={r.requestHeaders} />
          <h4>Corpo{r.requestSize > 0 ? ` · ${formatBytes(r.requestSize)}` : ""}</h4>
          {r.form && Object.keys(r.form).length > 0 && (
            <pre className="code json">{JSON.stringify(r.form, null, 2)}</pre>
          )}
          {r.files.length > 0 && (
            <ul className="files">
              {r.files.map((f, i) => (
                <li key={i}>
                  <Icon name="file" size={13} /> <code>{f.field}</code> {f.name} · {formatBytes(f.size)}
                </li>
              ))}
            </ul>
          )}
          {!r.form && <Body text={r.requestBody} truncated={r.requestBodyTruncated} empty="Esta requisição não tem corpo." />}
        </>
      ) : (
        <>
          <h4>Cabeçalhos</h4>
          <HeaderTable headers={r.responseHeaders} />
          <h4>Corpo · {formatBytes(r.responseSize)}</h4>
          <Body text={r.responseBody} truncated={r.responseBodyTruncated} empty="A resposta não tem corpo." />
        </>
      )}
    </div>
  );
}

// ------------------------------------------------------------------ cliente

function Client(props: {
  project: Project;
  state: ClientState;
  setState: (s: ClientState) => void;
  response: ApiResponse | null;
  failure: string | null;
  sending: boolean;
  onSend: () => void;
  history: HistoryItem[];
  onLoad: (s: ClientState) => void;
  onClearHistory: () => void;
  collection: Collection;
}) {
  const { project: p, state: s, response, collection: col } = props;
  const [tab, setTab] = useState<"headers" | "body">("headers");
  const [resTab, setResTab] = useState<"body" | "headers">("body");
  const set = (patch: Partial<ClientState>) => props.setState({ ...s, ...patch });
  const setRow = (id: number, patch: Partial<Row>) =>
    set({ headers: s.headers.map((r) => (r.id === id ? { ...r, ...patch } : r)) });

  const jsonError = useMemo(() => {
    if (s.bodyMode !== "json" || !s.body.trim()) return null;
    try {
      JSON.parse(s.body);
      return null;
    } catch (e) {
      return errMsg(e);
    }
  }, [s.bodyMode, s.body]);

  const req = buildRequest(p, s);
  const resBody = response && !response.binary ? pretty(response.body) : null;

  return (
    <div className="client">
      <form
        className="url-bar"
        onSubmit={(e) => {
          e.preventDefault();
          props.onSend();
        }}
      >
        <select aria-label="Método HTTP" value={s.method} onChange={(e) => set({ method: e.target.value })}>
          {METHODS.map((m) => (
            <option key={m}>{m}</option>
          ))}
        </select>
        <input
          className="grow mono-input"
          aria-label="URL"
          placeholder={`${p.url}/api/…  ou  /api/…`}
          value={s.url}
          onChange={(e) => set({ url: e.target.value })}
          spellCheck={false}
          autoCapitalize="off"
        />
        <button className="btn primary" disabled={props.sending || !s.url.trim()}>
          <Icon name="send" size={14} /> {props.sending ? "Enviando…" : "Enviar"}
        </button>
      </form>

      <div className="client-main">
      <div className="client-req">
      <div className="chips">
        <button className={`chip-btn ${tab === "headers" ? "active" : ""}`} onClick={() => setTab("headers")}>
          Cabeçalhos ({s.headers.filter((r) => r.enabled && r.key.trim()).length})
        </button>
        <button className={`chip-btn ${tab === "body" ? "active" : ""}`} onClick={() => setTab("body")}>
          Corpo{s.bodyMode !== "none" ? " •" : ""}
        </button>
        <span className="grow" />
        <label className="check inline">
          <input type="checkbox" checked={s.followRedirects} onChange={(e) => set({ followRedirects: e.target.checked })} />
          Seguir redirecionamentos
        </label>
      </div>

      {tab === "headers" ? (
        <div className="hdr-editor">
          {s.headers.map((r) => (
            <div className="hdr-row" key={r.id}>
              <input type="checkbox" checked={r.enabled} aria-label="Usar este cabeçalho" onChange={(e) => setRow(r.id, { enabled: e.target.checked })} />
              <input className="mono-input" placeholder="Nome" aria-label="Nome do cabeçalho" value={r.key} onChange={(e) => setRow(r.id, { key: e.target.value })} spellCheck={false} />
              <input className="mono-input" placeholder="Valor" aria-label="Valor do cabeçalho" value={r.value} onChange={(e) => setRow(r.id, { value: e.target.value })} spellCheck={false} />
              <button type="button" className="btn small ghost" aria-label="Remover cabeçalho" onClick={() => set({ headers: s.headers.filter((x) => x.id !== r.id) })}>
                <Icon name="trash" size={13} />
              </button>
            </div>
          ))}
          <div className="row">
            <button type="button" className="btn small" onClick={() => set({ headers: [...s.headers, row()] })}>
              <Icon name="plus" size={13} /> Cabeçalho
            </button>
            <button type="button" className="btn small ghost" onClick={() => set({ headers: [...s.headers, row("Authorization", "Bearer ")] })}>
              Authorization (Bearer)
            </button>
          </div>
        </div>
      ) : (
        <div className="body-editor">
          <div className="row">
            <select aria-label="Tipo do corpo" value={s.bodyMode} onChange={(e) => set({ bodyMode: e.target.value as BodyMode })}>
              <option value="none">Sem corpo</option>
              <option value="json">JSON</option>
              <option value="form">Formulário (a=1&amp;b=2)</option>
              <option value="text">Texto</option>
            </select>
            {s.bodyMode === "json" && (
              <button
                type="button"
                className="btn small"
                disabled={!!jsonError || !s.body.trim()}
                onClick={() => set({ body: pretty(s.body).text })}
              >
                Formatar JSON
              </button>
            )}
            {s.bodyMode !== "none" && <span className="muted small">O Content-Type é definido sozinho, se você não informar.</span>}
          </div>
          {s.bodyMode !== "none" && (
            <>
              <textarea
                className="code-input"
                aria-label="Corpo da requisição"
                rows={9}
                spellCheck={false}
                placeholder={s.bodyMode === "json" ? '{\n  "nome": "Ana"\n}' : ""}
                value={s.body}
                onChange={(e) => set({ body: e.target.value })}
              />
              {jsonError && <p className="err-text small">JSON inválido: {jsonError}</p>}
            </>
          )}
        </div>
      )}

      <div className="row">
        <button type="button" className="btn small ghost" onClick={() => copy(toCurl(req.method, req.url, req.headers, req.body))}>
          <Icon name="copy" size={13} /> Copiar como cURL
        </button>
      </div>

      </div>

      <section className="panel response">
        <h3>
          <Icon name="activity" /> Resposta
        </h3>
        {props.failure ? (
          <div className="banner err" role="alert">
            <Icon name="alert" />
            <div>{props.failure}</div>
          </div>
        ) : !response ? (
          <p className="muted small">{props.sending ? "Aguardando a resposta…" : "Envie uma requisição para ver a resposta aqui."}</p>
        ) : (
          <>
            <div className="res-meta">
              <span className={`status big ${statusClass(response.status)}`}>
                {response.status} {response.statusText}
              </span>
              <span className="muted small">
                {response.durationMs} ms · {formatBytes(response.size)}
              </span>
              <span className="grow" />
              <button className="btn small ghost" onClick={() => copy(resBody?.text ?? response.body)} disabled={response.binary}>
                <Icon name="copy" size={13} /> Copiar corpo
              </button>
            </div>
            <div className="chips">
              <button className={`chip-btn ${resTab === "body" ? "active" : ""}`} onClick={() => setResTab("body")}>
                Corpo
              </button>
              <button className={`chip-btn ${resTab === "headers" ? "active" : ""}`} onClick={() => setResTab("headers")}>
                Cabeçalhos ({response.headers.length})
              </button>
            </div>
            {resTab === "headers" ? (
              <HeaderTable headers={response.headers} />
            ) : response.binary ? (
              <p className="muted small">Conteúdo binário ({formatBytes(response.size)}), não exibido.</p>
            ) : response.body === "" ? (
              <p className="muted small">A resposta não tem corpo.</p>
            ) : (
              <>
                <pre className={`code ${resBody?.json ? "json" : ""}`}>{resBody?.text}</pre>
                {response.truncated && <p className="hint">Resposta cortada em 5 MB.</p>}
              </>
            )}
          </>
        )}
      </section>

      </div>

      <div className="client-lib">
      <section className="panel collection">
        <div className="row">
          <h3 className="grow">
            <Icon name="archive" /> Coleção <span className="count">{col.items.length}</span>
          </h3>
          <button type="button" className="btn small ghost" disabled={!col.ready} onClick={col.onImport}>
            <Icon name="download" size={13} /> Importar
          </button>
          <button type="button" className="btn small ghost" disabled={!col.ready} onClick={col.onExport}>
            <Icon name="external" size={13} /> Exportar
          </button>
        </div>
        <form
          className="save-bar"
          onSubmit={(e) => {
            e.preventDefault();
            col.onSave();
          }}
        >
          <input
            aria-label="Nome da requisição"
            placeholder={`Nome (ex.: ${req.method} ${(s.url.replace(p.url, "") || "/").slice(0, 28)})`}
            value={col.name}
            onChange={(e) => col.setName(e.target.value)}
          />
          {col.activeId && col.items.some((r) => r.id === col.activeId) && (
            <button type="button" className="btn" disabled={!col.ready} onClick={col.onUpdate}>
              Atualizar
            </button>
          )}
          <button className="btn primary" disabled={!col.ready || !s.url.trim()}>
            <Icon name="plus" size={14} /> Salvar como nova
          </button>
        </form>
        {col.note && <p className="hint ok-text">{col.note}</p>}
        {col.items.length === 0 ? (
          <p className="muted small">
            Salve as requisições que você usa sempre para reabrir com um clique, e exporte a coleção para compartilhar.
          </p>
        ) : (
          <ul className="history">
            {col.items.map((r) => (
              <li key={r.id} className="saved-item">
                <button type="button" className={`req-item ${col.activeId === r.id ? "active" : ""}`} onClick={() => col.onOpen(r)}>
                  <span className={`method m-${r.method.toLowerCase()}`}>{r.method}</span>
                  <span className="saved-name ellipsis" title={r.url}>
                    {r.name}
                  </span>
                  <span className="muted small ellipsis saved-url">{r.url}</span>
                </button>
                <button type="button" className="btn small ghost" aria-label={`Remover ${r.name}`} onClick={() => col.onDelete(r)}>
                  <Icon name="trash" size={13} />
                </button>
              </li>
            ))}
          </ul>
        )}
      </section>

      {props.history.length > 0 && (
        <section className="panel">
          <div className="row">
            <h3 className="grow">
              <Icon name="restore" /> Histórico
            </h3>
            <button className="btn small ghost" onClick={props.onClearHistory}>
              Limpar
            </button>
          </div>
          <ul className="history">
            {props.history.map((h, i) => (
              <li key={`${h.at}-${i}`}>
                <button className="req-item" onClick={() => props.onLoad(h.state)}>
                  <span className={`method m-${h.state.method.toLowerCase()}`}>{h.state.method}</span>
                  <span className="req-path ellipsis" title={h.state.url}>
                    {h.state.url.replace(p.url, "") || "/"}
                  </span>
                  <span className={`status ${h.status ? statusClass(h.status) : "s5xx"}`}>{h.status ?? "erro"}</span>
                </button>
              </li>
            ))}
          </ul>
        </section>
      )}
      </div>
    </div>
  );
}
