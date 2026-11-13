import { open, save } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useState } from "react";
import { BlueprintInfo, InstallProgress, Project, api, formatBytes } from "./api";
import { formatDate } from "./Backup";
import { askConfirm } from "./confirm";
import { ProgressView, isBusy } from "./PhpManager";
import { Icon, Skeleton, Spinner } from "./ui";

function errMsg(e: unknown) {
  return typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
}

const FILTERS = [{ name: "Blueprint do LocalTusk", extensions: ["gz"] }];

/** Salva o projeto aberto como blueprint (painel da visão geral do projeto). */
export function SaveBlueprintPanel(props: {
  project: Project;
  progress: Record<string, InstallProgress>;
  onOpenBlueprints: () => void;
  onError: (m: string) => void;
}) {
  const { project: p } = props;
  const progress = props.progress[`blueprint-${p.id}`];
  const working = isBusy(progress);
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [pending, setPending] = useState(false);
  const [saved, setSaved] = useState<BlueprintInfo | null>(null);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setPending(true);
    setSaved(null);
    try {
      const b = await api.createBlueprint(p.id, name, description);
      setSaved(b);
      setName("");
      setDescription("");
    } catch (err) {
      props.onError(errMsg(err));
    } finally {
      setPending(false);
    }
  }

  return (
    <section className="panel">
      <h3>
        <Icon name="blueprint" /> Salvar como blueprint
      </h3>
      <p className="muted small">
        Guarda os arquivos, o banco de dados e as versões de PHP e do banco deste projeto como um modelo, para criar novos
        projetos já prontos.
        {!p.running && " Se o projeto estiver parado, ele é ligado só durante o processo."}
      </p>

      {working || pending ? (
        <>
          <p className="small row nowrap" role="status">
            <Spinner size={13} /> Criando o blueprint…
          </p>
          {progress && <ProgressView p={progress} />}
        </>
      ) : (
        <form className="wp-form" onSubmit={submit}>
          <div className="field-row">
            <div className="field">
              <label className="field-label" htmlFor={`bp-name-${p.id}`}>Nome do blueprint</label>
              <input
                id={`bp-name-${p.id}`}
                value={name}
                maxLength={60}
                onChange={(e) => setName(e.target.value)}
                placeholder={`Ex.: ${p.name} base`}
              />
            </div>
            <div className="field">
              <label className="field-label" htmlFor={`bp-desc-${p.id}`}>
                Descrição <span className="muted">(opcional)</span>
              </label>
              <input
                id={`bp-desc-${p.id}`}
                value={description}
                maxLength={280}
                onChange={(e) => setDescription(e.target.value)}
                placeholder="Para que serve este modelo"
              />
            </div>
          </div>
          <div className="row">
            <button className="btn small primary" disabled={!name.trim()}>
              <Icon name="blueprint" size={13} /> Criar blueprint
            </button>
            <button type="button" className="btn small ghost" onClick={props.onOpenBlueprints}>
              Ver blueprints
            </button>
          </div>
        </form>
      )}

      {saved && !working && !pending && (
        <p className="small ok-text" role="status">
          Blueprint <strong>{saved.name}</strong> criado ({formatBytes(saved.size)}).{" "}
          <button className="link" onClick={props.onOpenBlueprints}>
            Ver blueprints
          </button>
        </p>
      )}
    </section>
  );
}

/** Tela com todos os blueprints: criar projeto a partir de um, compartilhar, importar e apagar. */
export default function BlueprintsView(props: {
  blueprintsDir: string;
  /** Já há um projeto sendo criado a partir de um blueprint. */
  creating: boolean;
  onCreate: (b: BlueprintInfo, projectName: string) => Promise<void>;
  onError: (m: string) => void;
}) {
  const [items, setItems] = useState<BlueprintInfo[] | null>(null);
  const [using, setUsing] = useState<string | null>(null);
  const [projectName, setProjectName] = useState("");
  const [notice, setNotice] = useState<string | null>(null);

  const load = useCallback(() => {
    api.listBlueprints().then(setItems).catch((e) => props.onError(errMsg(e)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  useEffect(load, [load]);

  function startUsing(b: BlueprintInfo) {
    setUsing(b.path);
    setProjectName(b.name);
    setNotice(null);
  }

  async function create(b: BlueprintInfo, e: React.FormEvent) {
    e.preventDefault();
    await props.onCreate(b, projectName);
  }

  async function importFile() {
    try {
      const selected = await open({ multiple: false, directory: false, defaultPath: props.blueprintsDir, filters: FILTERS });
      if (typeof selected !== "string") return;
      const b = await api.importBlueprint(selected);
      setNotice(`Blueprint "${b.name}" importado.`);
      load();
    } catch (e) {
      props.onError(errMsg(e));
    }
  }

  async function exportFile(b: BlueprintInfo) {
    try {
      const dest = await save({ defaultPath: b.fileName, filters: FILTERS });
      if (!dest) return;
      const written = await api.exportBlueprint(b.path, dest);
      setNotice(`Cópia de "${b.name}" salva em ${written}`);
    } catch (e) {
      props.onError(errMsg(e));
    }
  }

  async function remove(b: BlueprintInfo) {
    const ok = await askConfirm({
      title: `Apagar o blueprint "${b.name}"?`,
      message: "O modelo será removido. Os projetos já criados a partir dele não são afetados.",
      confirmLabel: "Apagar blueprint",
      danger: true,
    });
    if (!ok) return;
    try {
      await api.deleteBlueprint(b.path);
      if (using === b.path) setUsing(null);
    } catch (e) {
      props.onError(errMsg(e));
    }
    load();
  }

  return (
    <div className="project">
      <header className="project-head">
        <div className="title">
          <h1>Blueprints</h1>
          <div className="meta">Modelos de projeto: arquivos, banco de dados e versões prontos para reaproveitar.</div>
        </div>
        <div className="actions">
          <button className="btn" onClick={importFile}>
            <Icon name="download" /> Importar de um arquivo…
          </button>
          <button className="btn ghost" onClick={() => api.openBlueprintsDir().catch((e) => props.onError(errMsg(e)))}>
            <Icon name="folder" /> Abrir pasta
          </button>
        </div>
      </header>

      {notice && (
        <p className="small ok-text" role="status">
          {notice}
        </p>
      )}

      {items === null ? (
        <Skeleton kind="card" count={2} />
      ) : items.length === 0 ? (
        <section className="panel empty-state">
          <Icon name="blueprint" size={28} />
          <h3>Nenhum blueprint ainda</h3>
          <p className="muted small">
            Abra um projeto e use <strong>Salvar como blueprint</strong> na visão geral. Depois, é só criar novos projetos
            a partir dele em um clique. Também dá para importar um blueprint recebido de outra pessoa.
          </p>
        </section>
      ) : (
        <ul className="backup-list blueprint-list" aria-label="Blueprints">
          {items.map((b) => (
            <li key={b.path} className="blueprint-item">
              <div className="blueprint-row">
                <span className="release-icon">
                  <Icon name="blueprint" />
                </span>
                <div className="grow">
                  <strong className="ellipsis">{b.name}</strong>
                  {b.description && <div className="muted small">{b.description}</div>}
                  <div className="muted small mono">
                    {b.phpVersion ? `PHP ${b.phpVersion}` : "PHP mais recente"} ·{" "}
                    {b.dbEngine === "postgres" ? "PostgreSQL" : "MySQL"} {b.dbVersion || "mais recente"}
                    {b.hasWordpress ? " · WordPress" : ""} · {formatBytes(b.size)}
                  </div>
                  <div className="muted small">
                    de {b.sourceName} · {formatDate(b.createdAt)}
                  </div>
                </div>
                <button className="btn small primary" disabled={props.creating} onClick={() => startUsing(b)}>
                  <Icon name="plus" size={13} /> Criar projeto
                </button>
                <button className="btn small ghost" onClick={() => exportFile(b)} aria-label={`Compartilhar ${b.name}`} title="Salvar uma cópia para compartilhar">
                  <Icon name="upload" size={13} />
                </button>
                <button className="btn small ghost" disabled={props.creating} onClick={() => remove(b)} aria-label={`Apagar ${b.name}`} title="Apagar blueprint">
                  <Icon name="trash" size={13} />
                </button>
              </div>
              {using === b.path && (
                <form className="blueprint-use" onSubmit={(e) => create(b, e)}>
                  <label className="field-label" htmlFor="bp-project-name">Nome do novo projeto</label>
                  <div className="row">
                    <input
                      id="bp-project-name"
                      autoFocus
                      className="grow"
                      value={projectName}
                      onChange={(e) => setProjectName(e.target.value)}
                    />
                    <button className="btn small primary" disabled={props.creating || !projectName.trim()}>
                      {props.creating ? (
                        <>
                          <Spinner size={13} /> Criando…
                        </>
                      ) : (
                        "Criar"
                      )}
                    </button>
                    <button type="button" className="btn small ghost" onClick={() => setUsing(null)}>
                      Cancelar
                    </button>
                  </div>
                </form>
              )}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
