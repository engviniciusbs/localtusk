//! HTTPS local: a autoridade de certificação (CA) do Caddy, compartilhada por todos os
//! projetos, e a instalação dela como "confiável" no sistema.
//!
//! O Caddy emite o certificado de cada site (`tls internal`) a partir de uma CA própria,
//! guardada em `~/LocalTusk/ssl`. Como a pasta é a mesma para todos os projetos, o usuário
//! confia na CA uma única vez e todos os sites ficam com o cadeado verde.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Pasta de dados do Caddy (`XDG_DATA_HOME`), igual para todos os projetos.
pub fn data_home(base: &Path) -> PathBuf {
    base.join("ssl")
}

/// Certificado raiz da CA local. Existe depois que o Caddy emite o primeiro certificado.
pub fn root_cert(base: &Path) -> PathBuf {
    data_home(base).join("caddy").join("pki").join("authorities").join("local").join("root.crt")
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SslStatus {
    /// A CA já foi criada.
    pub ca_ready: bool,
    /// O sistema confia na CA (os navegadores não mostram aviso).
    pub trusted: bool,
    /// Caminho do certificado raiz, para instalar em outros aparelhos.
    pub ca_path: String,
    /// Dá para instalar a confiança automaticamente neste sistema.
    pub can_trust: bool,
}

pub fn status(base: &Path) -> SslStatus {
    let root = root_cert(base);
    let ca_ready = root.is_file();
    SslStatus {
        ca_ready,
        trusted: ca_ready && is_trusted(&root),
        ca_path: root.display().to_string(),
        can_trust: cfg!(target_os = "macos"),
    }
}

/// Garante que a CA exista. Se ainda não existe, sobe um Caddy descartável só com um
/// site `tls internal`, espera o certificado raiz aparecer e encerra.
pub fn ensure_ca(base: &Path, caddy: &Path) -> Result<PathBuf, String> {
    let root = root_cert(base);
    if root.is_file() {
        return Ok(root);
    }
    let home = data_home(base);
    std::fs::create_dir_all(&home).map_err(|e| e.to_string())?;
    let port = crate::project::ephemeral_ports(1).ok_or("sem portas livres")?[0];
    let cfg = home.join("ca-bootstrap.Caddyfile");
    std::fs::write(
        &cfg,
        format!(
            "{{\n\tadmin off\n\tskip_install_trust\n\tauto_https disable_redirects\n}}\n\n\
             https://localhost:{port} {{\n\tbind 127.0.0.1\n\ttls internal\n\trespond \"ok\"\n}}\n"
        ),
    )
    .map_err(|e| e.to_string())?;

    let mut child = Command::new(caddy)
        .args(["run", "--adapter", "caddyfile", "--config"])
        .arg(&cfg)
        .env("XDG_DATA_HOME", &home)
        .env("XDG_CONFIG_HOME", &home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("não foi possível iniciar o Caddy: {e}"))?;

    let start = Instant::now();
    let result = loop {
        if root.is_file() {
            break Ok(root.clone());
        }
        if let Ok(Some(status)) = child.try_wait() {
            break Err(format!("o Caddy encerrou ({status}) antes de criar o certificado"));
        }
        if start.elapsed() > Duration::from_secs(20) {
            break Err("tempo esgotado criando o certificado HTTPS".to_string());
        }
        std::thread::sleep(Duration::from_millis(150));
    };
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_file(&cfg);
    result
}

/// Escapa um texto para ficar entre aspas duplas num AppleScript.
fn applescript_quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Escapa um texto para uso entre aspas simples em shell.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Script do AppleScript que roda `command` no shell, pedindo a senha de administrador
/// (diálogo nativo do macOS, com Touch ID quando disponível).
fn admin_script(command: &str, prompt: &str, privileged: bool) -> String {
    let mut s = format!("do shell script {}", applescript_quote(command));
    if privileged {
        s.push_str(&format!(" with prompt {} with administrator privileges", applescript_quote(prompt)));
    }
    s
}

/// Comando que instala a CA como raiz confiável do sistema.
fn trust_command(root: &Path) -> String {
    format!(
        "/usr/bin/security add-trusted-cert -d -r trustRoot -k /Library/Keychains/System.keychain {}",
        sh_quote(&root.display().to_string())
    )
}

#[cfg(target_os = "macos")]
fn is_trusted(root: &Path) -> bool {
    Command::new("/usr/bin/security")
        .args(["verify-cert", "-p", "ssl", "-c"])
        .arg(root)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(not(target_os = "macos"))]
fn is_trusted(_root: &Path) -> bool {
    false
}

/// Faz o sistema confiar na CA local. No macOS, mostra o pedido de senha do sistema.
#[cfg(target_os = "macos")]
pub fn trust(base: &Path, caddy: &Path) -> Result<(), String> {
    let root = ensure_ca(base, caddy)?;
    if is_trusted(&root) {
        return Ok(());
    }
    let script = admin_script(
        &trust_command(&root),
        "O LocalTusk precisa da sua autorização para confiar no certificado HTTPS local.",
        true,
    );
    let out = Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|e| format!("osascript: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    if err.contains("-128") || err.to_lowercase().contains("cancel") {
        Err("Autorização cancelada. O HTTPS continua ligado, mas o navegador vai avisar que o certificado não é confiável.".into())
    } else {
        Err(format!("Não foi possível confiar no certificado: {}", err.trim()))
    }
}

#[cfg(not(target_os = "macos"))]
pub fn trust(base: &Path, caddy: &Path) -> Result<(), String> {
    let root = ensure_ca(base, caddy)?;
    Err(format!(
        "Neste sistema o certificado ainda precisa ser instalado manualmente. Ele está em {}",
        root.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aspas_do_applescript_escapam_barra_e_aspas() {
        assert_eq!(applescript_quote(r#"a"b\c"#), r#""a\"b\\c""#);
    }

    #[test]
    fn comando_de_confianca_protege_caminhos_com_espaco_e_aspas() {
        let cmd = trust_command(Path::new("/Users/ana maria/it's/root.crt"));
        assert!(cmd.starts_with("/usr/bin/security add-trusted-cert -d -r trustRoot -k /Library/Keychains/System.keychain "));
        assert!(cmd.ends_with(r"'/Users/ana maria/it'\''s/root.crt'"));
    }

    #[test]
    fn script_sem_privilegio_nao_pede_senha() {
        let s = admin_script("echo oi", "x", false);
        assert_eq!(s, r#"do shell script "echo oi""#);
        let s = admin_script("echo oi", "x", true);
        assert!(s.contains("with administrator privileges") && s.contains(r#"with prompt "x""#));
    }

    /// O texto montado para o AppleScript chega inteiro ao shell, mesmo com aspas e espaços.
    #[cfg(target_os = "macos")]
    #[test]
    fn osascript_entrega_o_comando_ao_shell() {
        let cmd = format!("printf %s {}", sh_quote("a b \"c\" 'd'"));
        let out = Command::new("/usr/bin/osascript").arg("-e").arg(admin_script(&cmd, "", false)).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim_end(), "a b \"c\" 'd'");
    }

    #[test]
    fn caminhos_da_ca_ficam_dentro_da_pasta_de_dados() {
        let base = Path::new("/tmp/x");
        assert!(root_cert(base).starts_with(data_home(base)));
        assert!(root_cert(base).ends_with("pki/authorities/local/root.crt"));
    }
}
