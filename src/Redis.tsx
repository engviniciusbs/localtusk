import { useState } from "react";
import { Project, ValkeyInstall, api } from "./api";
import { Copyable, Icon, Spinner, Switch } from "./ui";

function errMsg(e: unknown) {
  return typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
}

/**
 * Redis do projeto (cache, sessões, filas), servido pelo Valkey. Fica ligado só neste
 * computador (127.0.0.1). Ligar ou desligar reinicia o projeto se ele estiver rodando.
 */
export default function RedisPanel(props: {
  project: Project;
  valkeys: ValkeyInstall[];
  onManage: () => void;
  onChanged: () => void;
  onError: (m: string) => void;
}) {
  const { project: p } = props;
  const [working, setWorking] = useState(false);
  const installed = props.valkeys.length > 0;
  const locked = working || p.busy;

  async function toggle(next: boolean) {
    setWorking(true);
    try {
      await api.setRedis(p.id, next);
    } catch (e) {
      props.onError(errMsg(e));
    } finally {
      setWorking(false);
      props.onChanged();
    }
  }

  const url = `redis://127.0.0.1:${p.redisPort}`;

  return (
    <section className="panel wide">
      <h3>
        <Icon name="database" /> Redis
        {p.running && locked && (
          <span className="tag nowrap">
            <Spinner size={11} /> reiniciando…
          </span>
        )}
      </h3>

      <div className="access-item">
        <div className="access-head">
          <span className="release-icon">
            <Icon name="database" />
          </span>
          <div className="grow">
            <strong>Redis (Valkey)</strong>
            <div className="muted small">Cache, sessões e filas, com <code>redis-cli</code>, phpredis e Predis.</div>
          </div>
          {!installed && !p.redis ? (
            <button className="btn small" onClick={props.onManage}>
              <Icon name="download" size={13} /> Instalar
            </button>
          ) : (
            <Switch checked={p.redis} onChange={toggle} disabled={locked} label="Ligar o Redis" />
          )}
        </div>

        {p.redis && (
          <div className="access-body">
            <dl className="kv">
              <dt>Host</dt>
              <dd><Copyable value="127.0.0.1" label="host" /></dd>
              <dt>Porta</dt>
              <dd><Copyable value={String(p.redisPort)} label="porta" /></dd>
              <dt>URL</dt>
              <dd><Copyable value={url} label="URL do Redis" /></dd>
            </dl>
            <div className="hint">
              No PHP: <code>REDIS_HOST</code> <code>REDIS_PORT</code> <code>REDIS_URL</code>. No terminal do projeto, o{" "}
              <code>redis-cli</code> já aponta para ele.
            </div>
            {!p.running && <div className="hint">Inicie o projeto para o Redis subir junto.</div>}
            {!installed && (
              <div className="hint err-text">
                O Valkey não está mais instalado.{" "}
                <button className="link" onClick={props.onManage}>
                  Instalar
                </button>
              </div>
            )}
          </div>
        )}
      </div>
    </section>
  );
}
