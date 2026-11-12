import { useCallback, useEffect, useState } from "react";
import { Project, SslStatus, api } from "./api";
import { askConfirm } from "./confirm";
import { Copyable, Icon, Spinner, Switch } from "./ui";

function errMsg(e: unknown) {
  return typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
}

/**
 * Como o site é aberto: HTTPS com certificado local confiável (um botão, como no Local) e
 * acesso por outros aparelhos da mesma rede. As duas opções reiniciam o projeto se ele estiver rodando.
 */
export default function AccessPanel(props: {
  project: Project;
  onChanged: () => void;
  onError: (m: string) => void;
}) {
  const { project: p } = props;
  const [ssl, setSsl] = useState<SslStatus | null>(null);
  const [working, setWorking] = useState<null | "https" | "lan" | "trust">(null);

  const refreshSsl = useCallback(() => {
    api.sslStatus().then(setSsl).catch(() => setSsl(null));
  }, []);
  useEffect(refreshSsl, [refreshSsl, p.https]);

  const locked = working !== null || p.busy;

  async function toggleHttps(next: boolean) {
    setWorking("https");
    try {
      await api.setHttps(p.id, next);
      if (next) {
        // Já com o HTTPS ligado, pede a confiança no certificado (senha de administrador no macOS).
        // Se a pessoa cancelar, o HTTPS segue ligado e o botão "Confiar" fica à mostra.
        const status = await api.sslStatus().catch(() => null);
        if (!status?.trusted && status?.canTrust !== false) {
          setWorking("trust");
          try {
            setSsl(await api.trustCertificate());
          } catch (e) {
            props.onError(errMsg(e));
          }
        }
      }
    } catch (e) {
      props.onError(errMsg(e));
    } finally {
      setWorking(null);
      refreshSsl();
      props.onChanged();
    }
  }

  async function trust() {
    setWorking("trust");
    try {
      setSsl(await api.trustCertificate());
    } catch (e) {
      props.onError(errMsg(e));
    } finally {
      setWorking(null);
      refreshSsl();
    }
  }

  async function toggleLan(next: boolean) {
    if (next) {
      const ok = await askConfirm({
        title: "Liberar acesso pela rede?",
        message:
          "Quem estiver na mesma rede (por exemplo, o Wi-Fi de um café ou de um coworking) poderá abrir este site. O banco de dados, o Adminer Neo e o Mailpit continuam acessíveis só neste computador. Use em redes de confiança.",
        confirmLabel: "Liberar acesso",
      });
      if (!ok) return;
    }
    setWorking("lan");
    try {
      await api.setLan(p.id, next);
    } catch (e) {
      props.onError(errMsg(e));
    } finally {
      setWorking(null);
      props.onChanged();
    }
  }

  const restarting = p.running && locked && (working === "https" || working === "lan");

  return (
    <section className="panel wide">
      <h3>
        <Icon name="lock" /> Acesso ao site
        {restarting && (
          <span className="tag nowrap">
            <Spinner size={11} /> reiniciando…
          </span>
        )}
      </h3>

      <div className="access-grid">
        <div className="access-item">
          <div className="access-head">
            <span className="release-icon">
              <Icon name="lock" />
            </span>
            <div className="grow">
              <strong>HTTPS</strong>
              <div className="muted small">Abra o site com <code>https://</code> e cadeado, como em produção.</div>
            </div>
            <Switch checked={p.https} onChange={toggleHttps} disabled={locked} label="Habilitar HTTPS" />
          </div>

          {p.https && (
            <div className="access-body">
              <Copyable value={p.url} label="endereço HTTPS" />
              {working === "trust" ? (
                <div className="hint">
                  <Spinner size={12} /> Aguardando a sua autorização do macOS…
                </div>
              ) : ssl?.trusted ? (
                <div className="hint ok-text">
                  <Icon name="check" size={12} /> Certificado confiável neste computador.
                </div>
              ) : ssl?.canTrust === false ? (
                <div className="hint">
                  O certificado ainda precisa ser instalado manualmente neste sistema{ssl.caReady ? ` (${ssl.caPath})` : ""}.
                </div>
              ) : (
                <div className="trust-row">
                  <span className="hint">O navegador vai avisar que o site não é seguro até você confiar no certificado.</span>
                  <button className="btn small primary" disabled={locked} onClick={trust}>
                    <Icon name="lock" size={13} /> Confiar no certificado
                  </button>
                </div>
              )}
              {!p.running && <div className="hint">Inicie o projeto para abrir o endereço HTTPS.</div>}
            </div>
          )}
        </div>

        <div className="access-item">
          <div className="access-head">
            <span className="release-icon">
              <Icon name="wifi" />
            </span>
            <div className="grow">
              <strong>Acessível na rede</strong>
              <div className="muted small">Abra o site no celular ou em outro computador da mesma rede Wi-Fi.</div>
            </div>
            <Switch checked={p.lan} onChange={toggleLan} disabled={locked} label="Acessível na rede" />
          </div>

          {p.lan && (
            <div className="access-body">
              {p.lanUrls.length > 0 ? (
                <>
                  {p.lanUrls.map((u) => (
                    <Copyable key={u} value={u} label="endereço na rede" />
                  ))}
                  {!p.running && <div className="hint">Inicie o projeto para os endereços funcionarem.</div>}
                  <div className="hint">
                    Só o site fica aberto na rede. Se o macOS perguntar se o Caddy pode receber conexões, escolha Permitir.
                    Mudou de rede? Reinicie o projeto.
                  </div>
                </>
              ) : (
                <div className="hint">Este computador não está conectado a nenhuma rede agora.</div>
              )}
              {p.https && p.lanUrls.length > 0 && (
                <div className="hint">
                  No HTTPS, o outro aparelho precisa confiar no certificado local, senão o navegador avisa.{" "}
                  <button className="link" onClick={() => api.revealCa().catch((e) => props.onError(errMsg(e)))}>
                    Mostrar o certificado
                  </button>
                </div>
              )}
              {p.hasWordpress && (
                <div className="hint">
                  O WordPress redireciona para o endereço configurado (<code>{p.url}</code>), que só existe neste
                  computador. Em outro aparelho, a página pode voltar para ele.
                </div>
              )}
            </div>
          )}
        </div>
      </div>
    </section>
  );
}
