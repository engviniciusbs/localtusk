import { open } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useState } from "react";
import { BackupInfo, InstallProgress, Project, api, formatBytes } from "./api";
import { askConfirm } from "./confirm";
import { ProgressView, isBusy } from "./PhpManager";
import { Icon, Spinner } from "./ui";

function errMsg(e: unknown) {
  return typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
}

export function formatDate(secs: number) {
  return new Date(secs * 1000).toLocaleString("pt-BR", { dateStyle: "short", timeStyle: "short" });
}

/** Abre o seletor de arquivos já na pasta de backups. Devolve `null` se o usuário cancelar. */
export async function pickBackupFile(defaultPath?: string): Promise<string | null> {
  const selected = await open({
    multiple: false,
    directory: false,
    defaultPath,
    filters: [{ name: "Backup do LocalTusk", extensions: ["gz"] }],
  });
  return typeof selected === "string" ? selected : null;
}

/** Backup e restauração do projeto (arquivos + banco) em `~/LocalTusk/backups`. */
export default function BackupPanel(props: {
  project: Project;
  backupsDir: string;
  progress: Record<string, InstallProgress>;
  onChanged: () => void;
  onError: (m: string) => void;
}) {
  const { project: p } = props;
  const backupProgress = props.progress[`backup-${p.id}`];
  const restoreProgress = props.progress[`restore-${p.id}`];
  const creating = isBusy(backupProgress);
  const restoring = isBusy(restoreProgress);
  const busy = creating || restoring;
  const [pending, setPending] = useState(false);
  const [backups, setBackups] = useState<BackupInfo[]>([]);

  const load = useCallback(() => {
    api.listBackups(p.id).then(setBackups).catch(() => {});
  }, [p.id]);

  // Recarrega a lista ao abrir e sempre que uma operação termina.
  useEffect(load, [load, busy]);

  async function create() {
    setPending(true);
    try {
      await api.createBackup(p.id);
    } catch (e) {
      props.onError(errMsg(e));
    } finally {
      setPending(false);
      load();
    }
  }

  async function restore(path: string) {
    const ok = await askConfirm({
      title: `Restaurar o backup em "${p.name}"?`,
      message:
        "Os arquivos e o banco de dados atuais deste projeto serão substituídos pelos do backup. " +
        "Antes disso, o LocalTusk guarda um backup de segurança do estado atual.",
      confirmLabel: "Restaurar",
      danger: true,
    });
    if (!ok) return;
    setPending(true);
    try {
      const r = await api.restoreBackup(path, p.id);
      if (r.warning) props.onError(r.warning);
    } catch (e) {
      props.onError(errMsg(e));
    } finally {
      setPending(false);
      props.onChanged();
      load();
    }
  }

  async function restoreFromFile() {
    try {
      const path = await pickBackupFile(props.backupsDir);
      if (path) await restore(path);
    } catch (e) {
      props.onError(errMsg(e));
    }
  }

  async function remove(b: BackupInfo) {
    const ok = await askConfirm({
      title: "Apagar este backup?",
      message: `O arquivo ${b.fileName} será removido da pasta de backups.`,
      confirmLabel: "Apagar backup",
      danger: true,
    });
    if (!ok) return;
    try {
      await api.deleteBackup(b.path);
    } catch (e) {
      props.onError(errMsg(e));
    }
    load();
  }

  const active = creating ? backupProgress : restoreProgress;

  return (
    <section className="panel">
      <h3>
        <Icon name="archive" /> Backup e restauração
      </h3>
      <p className="muted small">
        Guarda os arquivos do site e o banco de dados num único arquivo em <code>~/LocalTusk/backups</code>.
        {!p.running && " Se o projeto estiver parado, ele é ligado só durante o backup."}
      </p>

      {busy || pending ? (
        <>
          <p className="small row nowrap" role="status">
            <Spinner size={13} /> {restoring ? "Restaurando o backup…" : "Criando o backup…"}
          </p>
          {active && <ProgressView p={active} />}
        </>
      ) : (
        <div className="row">
          <button className="btn small primary" onClick={create}>
            <Icon name="archive" size={13} /> Criar backup
          </button>
          <button className="btn small" onClick={restoreFromFile}>
            <Icon name="restore" size={13} /> Restaurar de um arquivo…
          </button>
          <button className="btn small ghost" onClick={() => api.openBackupsDir().catch((e) => props.onError(errMsg(e)))}>
            <Icon name="folder" size={13} /> Abrir pasta de backups
          </button>
        </div>
      )}

      {backups.length > 0 && (
        <ul className="backup-list" aria-label="Backups deste projeto">
          {backups.slice(0, 6).map((b) => (
            <li key={b.path} className="backup-item">
              <div className="grow">
                <strong className="ellipsis">{formatDate(b.createdAt)}</strong>
                <div className="muted small mono ellipsis" title={b.fileName}>
                  {b.fileName} · {formatBytes(b.size)}
                </div>
              </div>
              <button className="btn small" disabled={busy || pending} onClick={() => restore(b.path)}>
                <Icon name="restore" size={13} /> Restaurar
              </button>
              <button
                className="btn small ghost"
                disabled={busy || pending}
                onClick={() => remove(b)}
                aria-label={`Apagar o backup de ${formatDate(b.createdAt)}`}
                title="Apagar backup"
              >
                <Icon name="trash" size={13} />
              </button>
            </li>
          ))}
        </ul>
      )}
      {backups.length > 6 && <p className="hint">Mostrando os 6 mais recentes. Os demais estão na pasta de backups.</p>}
    </section>
  );
}
