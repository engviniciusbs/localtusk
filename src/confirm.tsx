import { useEffect, useRef } from "react";
import { createRoot } from "react-dom/client";
import { Icon } from "./ui";

/**
 * Confirmação dentro do app. O `window.confirm` do navegador não abre nada no WebView do
 * Tauri (devolve `false` na hora), então os botões destrutivos pareciam não fazer nada.
 */
export interface ConfirmOptions {
  title: string;
  message: string;
  /** Texto do botão que confirma (padrão: "Confirmar"). */
  confirmLabel?: string;
  /** Destaca a ação como destrutiva (botão vermelho). */
  danger?: boolean;
}

export function askConfirm(opts: ConfirmOptions): Promise<boolean> {
  return new Promise((resolve) => {
    const host = document.createElement("div");
    document.body.appendChild(host);
    const root = createRoot(host);
    const close = (answer: boolean) => {
      root.unmount();
      host.remove();
      resolve(answer);
    };
    root.render(<ConfirmDialog {...opts} onClose={close} />);
  });
}

function ConfirmDialog(props: ConfirmOptions & { onClose: (answer: boolean) => void }) {
  const { title, message, confirmLabel = "Confirmar", danger, onClose } = props;
  const cancelRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    // O foco começa em "Cancelar": Enter sem querer não apaga nada.
    const previous = document.activeElement as HTMLElement | null;
    cancelRef.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose(false);
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
      previous?.focus?.();
    };
  }, [onClose]);

  return (
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose(false)}>
      <div className="modal" role="alertdialog" aria-modal="true" aria-labelledby="modal-title" aria-describedby="modal-msg">
        <div className="modal-head">
          <span className={`modal-icon ${danger ? "danger" : ""}`}>
            <Icon name={danger ? "trash" : "alert"} size={18} />
          </span>
          <h2 id="modal-title">{title}</h2>
        </div>
        <p id="modal-msg">{message}</p>
        <div className="row modal-actions">
          <button ref={cancelRef} className="btn" onClick={() => onClose(false)}>
            Cancelar
          </button>
          <button className={`btn ${danger ? "danger solid" : "primary"}`} onClick={() => onClose(true)}>
            {confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
