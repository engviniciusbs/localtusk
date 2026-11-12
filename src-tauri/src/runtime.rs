//! Motor: gera as configs de cada projeto e controla os processos
//! (MySQL, PHP-FPM e Caddy) sem Docker.

use crate::extras;
use crate::mysql_manager;
use crate::project::{self, DB_PASSWORD, DB_USER, Project};
use crate::tools;
use std::fs::{self, File};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

pub struct Service {
    pub name: &'static str,
    /// Arquivo de log (em `logs/`) usado para explicar falhas.
    pub log: &'static str,
    pub child: Child,
}

/// O que ficou disponível quando o projeto foi iniciado.
#[derive(Clone, Default)]
pub struct RunInfo {
    /// Porta do site de administração do banco (Adminer Neo).
    pub admin_port: Option<u16>,
    pub adminneo: bool,
    /// Porta da interface web do Mailpit.
    pub mail_ui_port: Option<u16>,
}

pub struct Running {
    pub services: Vec<Service>,
    pub info: RunInfo,
}

impl Running {
    /// `true` se todos os serviços continuam vivos.
    pub fn is_alive(&mut self) -> bool {
        self.services
            .iter_mut()
            .all(|s| matches!(s.child.try_wait(), Ok(None)))
    }
}

struct Layout {
    app: PathBuf,
    conf: PathBuf,
    logs: PathBuf,
    run: PathBuf,
    mysql_data: PathBuf,
}

impl Layout {
    fn new(base: &Path, p: &Project) -> Self {
        let root = project::project_dir(base, &p.id);
        Layout {
            app: root.join("app"),
            conf: root.join("conf"),
            logs: root.join("logs"),
            run: root.join("run"),
            mysql_data: root.join("data").join("mysql"),
        }
    }

    fn docroot(&self, p: &Project) -> PathBuf {
        if p.docroot.is_empty() {
            self.app.clone()
        } else {
            self.app.join(&p.docroot)
        }
    }

    /// Sockets Unix têm limite de ~104 caracteres no macOS; se o caminho do
    /// projeto for longo, o socket do MySQL vai para `/tmp`.
    fn mysql_socket(&self, p: &Project) -> PathBuf {
        let preferred = self.run.join("mysql.sock");
        if preferred.as_os_str().len() < 90 {
            preferred
        } else {
            std::env::temp_dir().join(format!("localtusk-{}.sock", p.id))
        }
    }
}

/// Cria a estrutura de pastas e os arquivos de exemplo de um projeto novo.
pub fn scaffold(base: &Path, p: &Project) -> Result<(), String> {
    let l = Layout::new(base, p);
    for d in [&l.app, &l.conf, &l.logs, &l.run, &l.mysql_data.parent().unwrap().to_path_buf()] {
        fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    let docroot = l.docroot(p);
    fs::create_dir_all(&docroot).map_err(|e| e.to_string())?;
    let index = docroot.join("index.php");
    if !index.exists() {
        fs::write(&index, SAMPLE_INDEX).map_err(|e| e.to_string())?;
    }
    Ok(())
}

const SAMPLE_INDEX: &str = r#"<?php
$db = ['host' => getenv('DB_HOST'), 'port' => getenv('DB_PORT'), 'name' => getenv('DB_NAME'),
       'user' => getenv('DB_USER'), 'pass' => getenv('DB_PASSWORD')];
try {
    $pdo = new PDO("mysql:host={$db['host']};port={$db['port']};dbname={$db['name']}", $db['user'], $db['pass']);
    $dbStatus = 'conectado (MySQL ' . $pdo->query('SELECT VERSION()')->fetchColumn() . ')';
} catch (Throwable $e) {
    $dbStatus = 'erro: ' . $e->getMessage();
}
?><!doctype html>
<html lang="pt-br"><head><meta charset="utf-8"><title>LocalTusk</title>
<style>body{font:16px system-ui;max-width:640px;margin:10vh auto;padding:0 1rem;color:#222}
code{background:#eee;padding:2px 6px;border-radius:4px}</style></head>
<body>
<h1>🚀 Seu projeto está rodando no LocalTusk</h1>
<p>PHP <strong><?= PHP_VERSION ?></strong> &middot; Banco: <strong><?= htmlspecialchars($dbStatus) ?></strong></p>
<p>Edite <code>app/<?= htmlspecialchars(basename(__DIR__) === 'app' ? '' : basename(__DIR__) . '/') ?>index.php</code> para começar.</p>
<p>Variáveis disponíveis: <code>DB_HOST</code>, <code>DB_PORT</code>, <code>DB_NAME</code>, <code>DB_USER</code>, <code>DB_PASSWORD</code>.</p>
</body></html>
"#;

fn write(path: &Path, content: String) -> Result<(), String> {
    fs::write(path, content).map_err(|e| format!("{}: {e}", path.display()))
}

fn q(path: &Path) -> String {
    path.display().to_string()
}

struct MailCfg {
    bin: PathBuf,
    web: u16,
    smtp: u16,
}

/// Ferramentas extras ativas neste start.
struct Extras {
    tools_dir: PathBuf,
    admin_port: Option<u16>,
    mail: Option<MailCfg>,
}

fn write_configs(l: &Layout, p: &Project, fpm_port: u16, x: &Extras) -> Result<(), String> {
    let socket = l.mysql_socket(p);

    // --- MySQL ---
    write(
        &l.conf.join("my.cnf"),
        format!(
            "[mysqld]\n\
             datadir={data}\n\
             socket={sock}\n\
             pid-file={run}/mysql.pid\n\
             log-error={logs}/mysql.log\n\
             port={port}\n\
             bind-address=127.0.0.1\n\
             mysqlx=OFF\n\
             skip-name-resolve\n\
             performance_schema=OFF\n\
             max_connections=50\n\
             innodb_buffer_pool_size=128M\n",
            data = q(&l.mysql_data),
            sock = q(&socket),
            run = q(&l.run),
            logs = q(&l.logs),
            port = p.db_port,
        ),
    )?;
    write(
        &l.conf.join("mysql-init.sql"),
        format!(
            "ALTER USER 'root'@'localhost' IDENTIFIED BY '{pw}';\n\
             CREATE DATABASE IF NOT EXISTS `{db}`;\n\
             CREATE USER IF NOT EXISTS '{u}'@'%' IDENTIFIED BY '{pw}';\n\
             GRANT ALL PRIVILEGES ON *.* TO '{u}'@'%' WITH GRANT OPTION;\n",
            pw = DB_PASSWORD,
            u = DB_USER,
            db = p.db_name
        ),
    )?;

    // --- PHP ---
    // PHP estático não conhece o repositório de certificados do sistema.
    let cacert = ["/etc/ssl/cert.pem", "/etc/ssl/certs/ca-certificates.crt", "/etc/pki/tls/certs/ca-bundle.crt"]
        .into_iter()
        .find(|p| Path::new(p).is_file())
        .map(|p| format!("curl.cainfo={p}\nopenssl.cafile={p}\n"))
        .unwrap_or_default();
    // Com o Mailpit instalado, mail() do PHP entrega na caixa de testes do projeto.
    let mail_ini = x
        .mail
        .as_ref()
        .map(|m| {
            format!(
                "sendmail_path=\"'{}' sendmail -S 127.0.0.1:{}\"\n",
                m.bin.display(),
                m.smtp
            )
        })
        .unwrap_or_default();
    // Registra as requisições PHP do projeto para a aba "API" (inspetor).
    let capture = l.conf.join("capture.php");
    write(
        &capture,
        crate::apitools::capture_script(&l.logs.join(crate::apitools::REQUESTS_LOG), &x.tools_dir),
    )?;
    write(
        &l.conf.join("php.ini"),
        format!(
            "{cacert}{mail_ini}\
             auto_prepend_file={capture}\n\
             memory_limit=512M\n\
             upload_max_filesize=64M\n\
             post_max_size=64M\n\
             max_execution_time=120\n\
             display_errors=On\n\
             error_reporting=E_ALL\n\
             date.timezone=UTC\n\
             error_log={logs}/php-error.log\n\
             pdo_mysql.default_socket={sock}\n\
             mysqli.default_socket={sock}\n\
             mysqli.default_port={port}\n",
            logs = q(&l.logs),
            sock = q(&socket),
            port = p.db_port,
            cacert = cacert,
            mail_ini = mail_ini,
            capture = q(&capture)
        ),
    )?;
    let mut env_lines = format!(
        "env[DB_HOST] = 127.0.0.1\n\
         env[DB_PORT] = {}\n\
         env[DB_NAME] = {}\n\
         env[DB_USER] = {}\n\
         env[DB_PASSWORD] = {}\n",
        p.db_port, p.db_name, DB_USER, DB_PASSWORD
    );
    if let Some(m) = &x.mail {
        env_lines.push_str(&format!(
            "env[MAIL_HOST] = 127.0.0.1\n\
             env[MAIL_PORT] = {smtp}\n\
             env[SMTP_HOST] = 127.0.0.1\n\
             env[SMTP_PORT] = {smtp}\n",
            smtp = m.smtp
        ));
    }
    write(
        &l.conf.join("php-fpm.conf"),
        format!(
            "[global]\n\
             pid = {run}/php-fpm.pid\n\
             error_log = {logs}/php-fpm.log\n\
             daemonize = no\n\n\
             [www]\n\
             listen = 127.0.0.1:{fpm}\n\
             pm = ondemand\n\
             pm.max_children = 8\n\
             pm.process_idle_timeout = 10s\n\
             catch_workers_output = yes\n\
             clear_env = no\n\
             {env_lines}",
            run = q(&l.run),
            logs = q(&l.logs),
            fpm = fpm_port,
        ),
    )?;

    // --- Caddy ---
    let caddyfile = render_caddyfile(
        p,
        &q(&l.docroot(p)),
        &q(&l.logs),
        fpm_port,
        x.admin_port.map(|port| (port, q(&x.tools_dir))),
        &project::lan_hosts(),
    );
    write(&l.conf.join("Caddyfile"), caddyfile)
}

/// Caddyfile do projeto.
///
/// - HTTP em `:<porta>` (aceita qualquer nome de host, então também serve o IP da rede).
/// - Com HTTPS ligado, uma segunda porta com `tls internal` (certificado da CA local) para
///   `<id>.localhost`, `127.0.0.1` e, com a rede ligada, o IP e o nome `.local` do computador.
/// - Só escuta em `127.0.0.1`, a menos que o acesso pela rede esteja ligado (`0.0.0.0`).
/// - O Adminer Neo (`admin`) fica sempre só em `127.0.0.1`.
fn render_caddyfile(
    p: &Project,
    docroot: &str,
    logs: &str,
    fpm: u16,
    admin: Option<(u16, String)>,
    lan_hosts: &[String],
) -> String {
    let https = project::https_on(p);
    let bind = if p.lan { "0.0.0.0" } else { "127.0.0.1" };

    let mut out = String::from("{\n\tadmin off\n");
    if https {
        // Mantém a emissão de certificados, mas sem redirecionar HTTP -> HTTPS (a porta 80
        // não é usada) e sem tentar instalar a CA sozinho: isso é um botão do app.
        out.push_str("\tauto_https disable_redirects\n\tskip_install_trust\n");
    } else {
        out.push_str("\tauto_https off\n");
    }
    out.push_str("}\n\n");

    out.push_str(&format!(
        "(app) {{\n\
         \troot * \"{docroot}\"\n\
         \tencode gzip\n\
         \tphp_fastcgi 127.0.0.1:{fpm}\n\
         \tfile_server\n\
         \tlog {{\n\
         \t\toutput file \"{logs}/access.log\"\n\
         \t}}\n\
         }}\n\n\
         :{port} {{\n\
         \tbind {bind}\n\
         \timport app\n\
         }}\n",
        port = p.http_port,
    ));

    if https {
        let mut hosts = vec![project::site_host(p), "127.0.0.1".to_string()];
        if p.lan {
            hosts.extend(lan_hosts.iter().cloned());
        }
        let addrs: Vec<String> = hosts.iter().map(|h| format!("https://{h}:{}", p.https_port)).collect();
        out.push_str(&format!(
            "\n{addrs} {{\n\
             \tbind {bind}\n\
             \ttls internal\n\
             \timport app\n\
             }}\n",
            addrs = addrs.join(", "),
        ));
    }

    // Site de administração do banco: a raiz é a pasta das ferramentas, então
    // /adminneo/ aponta direto para a pasta (sem reescrever URLs).
    if let Some((port, tools)) = admin {
        out.push_str(&format!(
            "\n:{port} {{\n\
             \tbind 127.0.0.1\n\
             \troot * \"{tools}\"\n\
             \t@allowed path /adminneo/*\n\
             \thandle @allowed {{\n\
             \t\tphp_fastcgi 127.0.0.1:{fpm} {{\n\
             \t\t\tenv PHP_VALUE \"display_errors=0\"\n\
             \t\t}}\n\
             \t\tfile_server\n\
             \t}}\n\
             \trespond \"Not found\" 404\n\
             }}\n",
        ));
    }
    out
}

fn log_file(l: &Layout, name: &str) -> Result<File, String> {
    File::options()
        .create(true)
        .append(true)
        .open(l.logs.join(name))
        .map_err(|e| e.to_string())
}

fn spawn(l: &Layout, mut cmd: Command, log: &str) -> Result<Child, String> {
    let out = log_file(l, log)?;
    let err = out.try_clone().map_err(|e| e.to_string())?;
    cmd.stdin(Stdio::null())
        .stdout(Stdio::from(out))
        .stderr(Stdio::from(err))
        .spawn()
        .map_err(|e| format!("não foi possível iniciar {:?}: {e}", cmd.get_program()))
}

/// `true` se o banco do projeto já foi criado (a versão do MySQL fica "travada" a partir daí).
pub fn db_initialized(base: &Path, id: &str) -> bool {
    let data = project::project_dir(base, id).join("data").join("mysql");
    data.join("mysql.ibd").exists() || data.join("mysql").exists()
}

fn mysql_initialized(l: &Layout) -> bool {
    l.mysql_data.join("mysql.ibd").exists() || l.mysql_data.join("mysql").exists()
}

fn init_mysql(l: &Layout, mysqld: &Path) -> Result<(), String> {
    fs::create_dir_all(&l.mysql_data).map_err(|e| e.to_string())?;
    let log = log_file(l, "mysql-init.log")?;
    let status = Command::new(mysqld)
        .arg(format!("--defaults-file={}", q(&l.conf.join("my.cnf"))))
        .arg("--initialize-insecure")
        .arg(format!("--init-file={}", q(&l.conf.join("mysql-init.sql"))))
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone().map_err(|e| e.to_string())?))
        .stderr(Stdio::from(log))
        .status()
        .map_err(|e| format!("mysqld: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        // Evita deixar um datadir pela metade, que impediria uma nova tentativa.
        let _ = fs::remove_dir_all(&l.mysql_data);
        let log = tail(&l.logs.join("mysql-init.log"), 6);
        Err(format!("falha ao inicializar o MySQL ({status})\n\n{log}"))
    }
}

/// Últimas linhas de um log, para incluir nas mensagens de erro.
fn tail(path: &Path, n: usize) -> String {
    let text = fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

fn wait_port(port: u16, timeout: Duration, running: &mut Running, logs: &Path, service: &str) -> Result<(), String> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let start = Instant::now();
    loop {
        if TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok() {
            return Ok(());
        }
        for s in running.services.iter_mut() {
            if let Ok(Some(status)) = s.child.try_wait() {
                let log = tail(&logs.join(s.log), 6);
                return Err(format!(
                    "{} encerrou inesperadamente ({status}){}",
                    s.name,
                    if log.is_empty() { String::new() } else { format!("\n\n{log}") }
                ));
            }
        }
        if start.elapsed() > timeout {
            let log = running
                .services
                .iter()
                .find(|s| s.name == service)
                .map(|s| tail(&logs.join(s.log), 6))
                .unwrap_or_default();
            return Err(format!(
                "tempo esgotado aguardando {service} na porta {port}{}",
                if log.is_empty() { String::new() } else { format!("\n\n{log}") }
            ));
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

/// Sobe MySQL, PHP-FPM e Caddy do projeto. Bloqueante.
pub fn start(base: &Path, p: &Project) -> Result<Running, String> {
    // Serviços de uma sessão anterior (app encerrado à força, crash...) segurariam o
    // banco e as portas do projeto.
    reap_orphans(base, &p.id);
    let l = Layout::new(base, p);
    scaffold(base, p)?;
    fs::create_dir_all(crate::ssl::data_home(base)).map_err(|e| e.to_string())?;

    let php = tools::pick_php(base, &p.php_version)
        .ok_or("PHP não encontrado. Instale uma versão em \"PHP e ferramentas\".")?;
    let caddy = extras::caddy_bin(base)
        .or_else(|| tools::find("caddy", base))
        .ok_or("Caddy não encontrado. Instale em \"PHP e ferramentas\".")?;
    let mysqld = mysql_manager::pick(base, &p.db_version)
        .ok_or("MySQL não encontrado. Instale uma versão em \"PHP e ferramentas\".")?
        .mysqld;

    let has_neo = extras::is_installed(base, "adminneo");
    let mailpit = extras::mailpit_bin(base);
    let ports = project::ephemeral_ports(4).ok_or("sem portas livres")?;
    let fpm_port = ports[0];
    let x = Extras {
        tools_dir: extras::tools_root(base),
        admin_port: has_neo.then_some(ports[1]),
        mail: mailpit.map(|bin| MailCfg {
            bin,
            web: ports[2],
            smtp: ports[3],
        }),
    };

    write_configs(&l, p, fpm_port, &x)?;
    if !mysql_initialized(&l) {
        init_mysql(&l, &mysqld)?;
    }

    let mut running = Running {
        services: vec![],
        info: RunInfo {
            admin_port: x.admin_port,
            adminneo: has_neo,
            mail_ui_port: x.mail.as_ref().map(|m| m.web),
        },
    };

    let result = (|| -> Result<(), String> {
        if let Some(m) = &x.mail {
            let mut c = Command::new(&m.bin);
            c.arg("--listen")
                .arg(format!("127.0.0.1:{}", m.web))
                .arg("--smtp")
                .arg(format!("127.0.0.1:{}", m.smtp))
                .arg("--database")
                .arg(l.mysql_data.parent().unwrap().join("mailpit.db"))
                .arg("--disable-version-check");
            running.services.push(Service {
                name: "Mailpit",
                log: "mailpit.log",
                child: spawn(&l, c, "mailpit.log")?,
            });
        }

        let mut c = Command::new(&mysqld);
        c.arg(format!("--defaults-file={}", q(&l.conf.join("my.cnf"))));
        running.services.push(Service {
            name: "MySQL",
            log: "mysql.log",
            child: spawn(&l, c, "mysql-stdout.log")?,
        });

        let mut c = Command::new(&php.fpm);
        c.arg("-F")
            .arg("-y")
            .arg(l.conf.join("php-fpm.conf"))
            .arg("-c")
            .arg(l.conf.join("php.ini"));
        running.services.push(Service {
            name: "PHP-FPM",
            log: "php-fpm.log",
            child: spawn(&l, c, "php-fpm-stdout.log")?,
        });

        let mut c = Command::new(&caddy);
        c.arg("run")
            .arg("--config")
            .arg(l.conf.join("Caddyfile"))
            .arg("--adapter")
            .arg("caddyfile")
            // Dados do Caddy (a CA do HTTPS) compartilhados entre os projetos.
            .env("XDG_DATA_HOME", crate::ssl::data_home(base))
            .env("XDG_CONFIG_HOME", &l.run);
        running.services.push(Service {
            name: "Caddy",
            log: "caddy.log",
            child: spawn(&l, c, "caddy.log")?,
        });

        wait_port(p.db_port, Duration::from_secs(90), &mut running, &l.logs, "MySQL")?;
        wait_port(fpm_port, Duration::from_secs(20), &mut running, &l.logs, "PHP-FPM")?;
        wait_port(p.http_port, Duration::from_secs(20), &mut running, &l.logs, "Caddy")?;
        if project::https_on(p) {
            wait_port(p.https_port, Duration::from_secs(20), &mut running, &l.logs, "Caddy")?;
        }
        if let Some(port) = x.admin_port {
            wait_port(port, Duration::from_secs(20), &mut running, &l.logs, "Caddy")?;
        }
        if let Some(m) = &x.mail {
            wait_port(m.web, Duration::from_secs(20), &mut running, &l.logs, "Mailpit")?;
        }
        Ok(())
    })();

    match result {
        Ok(()) => Ok(running),
        Err(e) => {
            stop(&mut running);
            Err(e)
        }
    }
}

/// Linhas `(pid, comando)` dos processos do projeto que o LocalTusk sobe
/// (mysqld, php-fpm, caddy, mailpit). Identifica pelo caminho da pasta do projeto
/// na linha de comando.
#[cfg(unix)]
pub fn project_processes(base: &Path, id: &str) -> Vec<(i32, String)> {
    let needle = format!("{}/", project::project_dir(base, id).display());
    let Ok(out) = Command::new("ps").args(["-axo", "pid=,command="]).output() else {
        return vec![];
    };
    parse_ps(&String::from_utf8_lossy(&out.stdout), &needle, std::process::id() as i32)
}

/// Filtra a listagem do `ps` (`pid comando` por linha): só mysqld, caddy, mailpit e php-fpm
/// cuja linha de comando cita a pasta do projeto (`needle`). Ignora o próprio processo (`me`).
fn parse_ps(listing: &str, needle: &str, me: i32) -> Vec<(i32, String)> {
    listing
        .lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let (pid, cmd) = line.split_once(' ')?;
            let pid: i32 = pid.parse().ok()?;
            let cmd = cmd.trim_start();
            let exe = cmd.split_whitespace().next().unwrap_or("");
            let name = exe.rsplit('/').next().unwrap_or("");
            let ours = matches!(name, "mysqld" | "caddy" | "mailpit") || cmd.starts_with("php-fpm:");
            (pid != me && ours && cmd.contains(needle)).then(|| (pid, cmd.to_string()))
        })
        .collect()
}

/// Encerra serviços do projeto que ficaram rodando fora do controle do app.
#[cfg(unix)]
pub fn reap_orphans(base: &Path, id: &str) -> usize {
    kill_processes(&project_processes(base, id))
}

/// Encerra os processos informados (SIGTERM, depois SIGKILL após 20s). Retorna quantos eram.
#[cfg(unix)]
pub fn kill_processes(found: &[(i32, String)]) -> usize {
    for (pid, _) in found {
        unsafe {
            libc::kill(*pid, libc::SIGTERM);
        }
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    for (pid, _) in found {
        while unsafe { libc::kill(*pid, 0) } == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
        if unsafe { libc::kill(*pid, 0) } == 0 {
            unsafe {
                libc::kill(*pid, libc::SIGKILL);
            }
        }
    }
    found.len()
}

#[cfg(not(unix))]
pub fn reap_orphans(_base: &Path, _id: &str) -> usize {
    0
}

#[cfg(not(unix))]
pub fn project_processes(_base: &Path, _id: &str) -> Vec<(i32, String)> {
    vec![]
}

#[cfg(not(unix))]
pub fn kill_processes(_found: &[(i32, String)]) -> usize {
    0
}

#[cfg(unix)]
fn terminate(child: &Child) {
    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }
}

#[cfg(not(unix))]
fn terminate(_child: &Child) {}

/// Para os serviços em ordem inversa (Caddy, PHP-FPM, MySQL), de forma graciosa.
pub fn stop(running: &mut Running) {
    for s in running.services.iter_mut().rev() {
        terminate(&s.child);
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            match s.child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(100))
                }
                _ => {
                    let _ = s.child.kill();
                    let _ = s.child.wait();
                    break;
                }
            }
        }
    }
    running.services.clear();
}

pub const LOG_FILES: &[&str] = &[
    "access.log",
    "caddy.log",
    "php-error.log",
    "php-fpm.log",
    "mysql.log",
    "mailpit.log",
    "wp-cli.log",
];

/// Últimas `max_lines` linhas de um log do projeto.
pub fn read_log(base: &Path, id: &str, file: &str, max_lines: usize) -> Result<String, String> {
    if !LOG_FILES.contains(&file) {
        return Err("log desconhecido".into());
    }
    let path = project::project_dir(base, id).join("logs").join(file);
    let text = fs::read_to_string(&path).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(max_lines);
    Ok(lines[start..].join("\n"))
}

/// Pasta pública do projeto (onde fica o `index.php`, ou o WordPress).
pub fn docroot_dir(base: &Path, p: &Project) -> PathBuf {
    Layout::new(base, p).docroot(p)
}

pub fn app_dir(base: &Path, id: &str) -> PathBuf {
    project::project_dir(base, id).join("app")
}

#[cfg(test)]
mod tests {
    /// Só serviços nossos, do projeto certo, são tratados como órfãos.
    #[test]
    fn reconhece_servicos_orfaos_do_projeto() {
        let needle = "/Users/ana/LocalTusk/projects/meu-site/";
        let listing = "\
  101 /opt/homebrew/bin/mysqld --defaults-file=/Users/ana/LocalTusk/projects/meu-site/conf/my.cnf
  102 php-fpm: master process (/Users/ana/LocalTusk/projects/meu-site/conf/php-fpm.conf)
  103 /Users/ana/LocalTusk/bin/tools/caddy/caddy run --config /Users/ana/LocalTusk/projects/meu-site/conf/Caddyfile --adapter caddyfile
  104 /Users/ana/LocalTusk/bin/tools/mailpit/mailpit --database /Users/ana/LocalTusk/projects/meu-site/data/mailpit.db
  105 php-fpm: pool www
  106 /opt/homebrew/bin/mysqld --defaults-file=/Users/ana/LocalTusk/projects/outro-site/conf/my.cnf
  107 /usr/bin/vim /Users/ana/LocalTusk/projects/meu-site/conf/my.cnf
  108 php wp-cli.phar --path=/Users/ana/LocalTusk/projects/meu-site/app/public core version
  109 /Users/ana/LocalTusk/projects/meu-site/conf/shims/mysql --version
  110 /opt/homebrew/bin/mysqld --defaults-file=/Users/ana/LocalTusk/projects/meu-site-2/conf/my.cnf
  999 /opt/homebrew/bin/mysqld --defaults-file=/Users/ana/LocalTusk/projects/meu-site/conf/my.cnf
";
        let pids = |me| parse_ps(listing, needle, me).into_iter().map(|(p, _)| p).collect::<Vec<_>>();
        // mysqld, php-fpm (master), caddy e mailpit do projeto; nada de editor, WP-CLI ou outros projetos.
        assert_eq!(pids(0), vec![101, 102, 103, 104, 999]);
        // O próprio processo do app nunca entra.
        assert_eq!(pids(999), vec![101, 102, 103, 104]);
        assert!(parse_ps(listing, "/Users/ana/LocalTusk/projects/nenhum/", 0).is_empty());
    }

    use super::*;
    use crate::download::InstallProgress;
    use std::io::{Read, Write};

    fn http_get(port: u16) -> String {
        http_get_path(port, "/")
    }

    fn http_get_path(port: u16, path: &str) -> String {
        http_request(port, "GET", path, "", "")
    }

    fn http_request(port: u16, method: &str, path: &str, extra_headers: &str, body: &str) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(15))).unwrap();
        // Sem um `Host:` próprio nos cabeçalhos extras, usa localhost:porta.
        let host = if extra_headers.starts_with("Host:") { String::new() } else { format!("Host: localhost:{port}\r\n") };
        write!(
            s,
            "{method} {path} HTTP/1.0\r\n{host}{extra_headers}Content-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        out
    }

    /// Teste ponta a ponta: precisa de PHP-FPM, Caddy e MySQL instalados.
    /// Rode com: `cargo test -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn start_serve_stop() {
        let base = std::env::temp_dir().join(format!("localtusk-test-{}", project::now()));
        let http = project::free_port(18000, &Default::default()).unwrap();
        let db = project::free_port(13307, &Default::default()).unwrap();
        let p = Project {
            id: "teste".into(),
            name: "Teste".into(),
            docroot: "".into(),
            php_version: "".into(),
            db_version: "".into(),
            http_port: http,
            db_port: db,
            db_name: "teste".into(),
            created_at: 0,
            ..Default::default()
        };
        let mut running = start(&base, &p).expect("start falhou");
        assert!(running.is_alive());
        let body = http_get(http);
        println!("{body}");
        assert!(body.contains("200 OK"), "resposta inesperada");
        assert!(body.contains("conectado"), "PHP não conectou ao MySQL");
        stop(&mut running);
        assert!(project::port_is_free(http), "Caddy ainda no ar");
        assert!(project::port_is_free(db), "MySQL ainda no ar");
        let _ = fs::remove_dir_all(&base);
    }

    /// O inspetor registra o que o PHP recebeu e respondeu, sem alterar a resposta ao cliente.
    /// Usa o PHP, o MySQL e o Caddy do sistema. Rode com:
    /// `cargo test --lib api_inspector -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn api_inspector_end_to_end() {
        let base = std::env::temp_dir().join(format!("localtusk-test-api-{}", project::now()));
        let http = project::free_port(18200, &Default::default()).unwrap();
        let db = project::free_port(13500, &Default::default()).unwrap();
        let p = Project {
            id: "api".into(),
            name: "API".into(),
            docroot: "public".into(),
            php_version: "".into(),
            db_version: "".into(),
            http_port: http,
            db_port: db,
            db_name: "api".into(),
            created_at: 0,
            ..Default::default()
        };
        scaffold(&base, &p).unwrap();
        fs::create_dir_all(docroot_dir(&base, &p)).unwrap();
        fs::write(
            docroot_dir(&base, &p).join("index.php"),
            "<?php\nheader('Content-Type: application/json');\nheader('X-Resposta: ok');\nhttp_response_code(201);\n\
             echo json_encode(['metodo' => $_SERVER['REQUEST_METHOD'], 'corpo' => file_get_contents('php://input'), 'ç' => 'ã']);\n",
        )
        .unwrap();
        let mut running = start(&base, &p).expect("start falhou");

        // 1) POST com JSON e um cabeçalho próprio.
        let json = r#"{"nome":"Ana","idade":30}"#;
        let r1 = http_request(
            http,
            "POST",
            "/index.php?x=1",
            "X-Teste: ola\r\nContent-Type: application/json\r\n",
            json,
        );
        assert!(r1.contains("201 Created") && r1.contains("X-Resposta: ok"), "resposta alterada: {r1}");
        assert!(r1.contains(r#""metodo":"POST""#) && r1.contains("Ana"), "o PHP não recebeu o corpo: {r1}");
        // 2) GET simples e 3) formulário urlencoded.
        http_request(http, "GET", "/index.php?q=busca", "", "");
        http_request(http, "POST", "/index.php", "Content-Type: application/x-www-form-urlencoded\r\n", "a=1&b=2");

        // O registro é gravado no fim da requisição; espera as 3 entradas.
        let mut entries = vec![];
        for _ in 0..40 {
            entries = crate::apitools::read_requests(&base, "api", 0).unwrap().entries;
            if entries.len() >= 3 {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert_eq!(entries.len(), 3, "esperava 3 requisições registradas: {entries:?}");

        let first = &entries[0];
        assert_eq!(first["method"], "POST");
        assert_eq!(first["uri"], "/index.php?x=1");
        assert_eq!(first["status"], 201);
        assert_eq!(first["requestBody"], json);
        assert_eq!(first["requestBodyTruncated"], false);
        let has = |list: &serde_json::Value, name: &str, value: &str| {
            list.as_array().unwrap().iter().any(|h| {
                h[0].as_str().unwrap().eq_ignore_ascii_case(name) && h[1].as_str().unwrap() == value
            })
        };
        assert!(has(&first["requestHeaders"], "X-Teste", "ola"), "{}", first["requestHeaders"]);
        assert!(has(&first["responseHeaders"], "X-Resposta", "ok"), "{}", first["responseHeaders"]);
        assert!(first["responseBody"].as_str().unwrap().contains("\"metodo\":\"POST\""));
        assert!(first["durationMs"].as_f64().unwrap() >= 0.0);
        assert_eq!(entries[1]["method"], "GET");
        assert_eq!(entries[1]["uri"], "/index.php?q=busca");
        assert_eq!(entries[2]["requestBody"], "a=1&b=2");

        // O cliente de API conversa com o mesmo projeto.
        let resp = crate::apitools::send(&crate::apitools::ApiRequest {
            method: "PUT".into(),
            url: format!("http://127.0.0.1:{http}/index.php"),
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: Some(r#"{"a":1}"#.into()),
            follow_redirects: false,
            timeout_secs: Some(10),
        })
        .expect("cliente");
        assert_eq!(resp.status, 201);
        assert!(resp.body.contains(r#""metodo":"PUT""#), "{}", resp.body);

        // Limpar zera o registro.
        crate::apitools::clear_requests(&base, "api").unwrap();
        assert!(crate::apitools::read_requests(&base, "api", 0).unwrap().entries.is_empty());

        stop(&mut running);
        let _ = fs::remove_dir_all(&base);
    }

    /// Baixa um PHP 8.1 de verdade (~36 MB), sobe um projeto com ele e confere a versão.
    /// Rode com: `cargo test --lib install_and_serve -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn install_and_serve_old_php() {
        let base = std::env::temp_dir().join(format!("localtusk-test-dl-{}", project::now()));

        let cat = crate::php_manager::catalog(&base).expect("catálogo");
        println!("{} versões no catálogo", cat.len());
        let target = cat.iter().find(|r| r.minor == "8.1" && r.latest).expect("8.1 no catálogo");
        println!("instalando PHP {}", target.version);

        crate::php_manager::install(&base, &target.version, &|p| {
            if p.stage != "download" || p.downloaded == p.total {
                println!("  [{}] {}/{}", p.stage, p.downloaded, p.total);
            }
        })
        .expect("install falhou");

        let installs = tools::php_installs(&base);
        assert!(installs.iter().any(|i| i.managed && i.full_version == target.version));

        let http = project::free_port(18100, &Default::default()).unwrap();
        let db = project::free_port(13400, &Default::default()).unwrap();
        let p = Project {
            id: "velho".into(),
            name: "Velho".into(),
            docroot: "".into(),
            php_version: "8.1".into(),
            db_version: "".into(),
            http_port: http,
            db_port: db,
            db_name: "velho".into(),
            created_at: 0,
            ..Default::default()
        };
        let mut running = start(&base, &p).expect("start falhou");
        let body = http_get(http);
        println!("{}", body.lines().take(12).collect::<Vec<_>>().join("\n"));
        assert!(body.contains(&format!("X-Powered-By: PHP/{}", target.version)), "não está usando o PHP baixado");
        assert!(body.contains("conectado"), "PHP baixado não conectou ao MySQL");
        stop(&mut running);

        crate::php_manager::uninstall(&base, &target.version).unwrap();
        assert!(!tools::php_installs(&base).iter().any(|i| i.managed));
        let _ = fs::remove_dir_all(&base);
    }

    /// Instala Adminer Neo e Mailpit de verdade, sobe um projeto e testa cada um.
    /// Rode com: `cargo test --lib extras_end_to_end -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn extras_end_to_end() {
        let base = std::env::temp_dir().join(format!("localtusk-test-extras-{}", project::now()));
        for id in extras::IDS {
            extras::install(&base, id, &|p| {
                if p.stage != "download" {
                    println!("  [{}] {}", p.id, p.stage);
                }
            })
            .unwrap_or_else(|e| panic!("instalar {id}: {e}"));
            println!("{id} instalado: {:?}", extras::status(&base).iter().find(|s| s.id == *id).unwrap().version);
        }

        let http = project::free_port(18200, &Default::default()).unwrap();
        let db = project::free_port(13500, &Default::default()).unwrap();
        let p = Project {
            id: "extras".into(),
            name: "Extras".into(),
            docroot: "".into(),
            php_version: "".into(),
            db_version: "".into(),
            http_port: http,
            db_port: db,
            db_name: "extras".into(),
            created_at: 0,
            ..Default::default()
        };
        let mut running = start(&base, &p).expect("start falhou");
        let info = running.info.clone();
        let admin = info.admin_port.expect("sem porta admin");
        let mail_ui = info.mail_ui_port.expect("sem mailpit");

        // Adminer Neo
        let neo = http_get_path(admin, "/adminneo/");
        println!("--- Adminer Neo: {}", neo.lines().next().unwrap_or(""));
        let forms: Vec<&str> = neo.split("name=\"").skip(1).map(|x| x.split('"').next().unwrap()).collect();
        println!("campos do formulário: {forms:?}");
        println!("server prefill presente: {}", neo.contains(&format!("127.0.0.1:{db}")));
        assert!(neo.starts_with("HTTP/1.0 200"));

        // Login automático do Adminer Neo, como o navegador faria: GET (cookie) + POST (login)
        let login_page = http_get_path(admin, "/adminneo/login.php");
        assert!(login_page.contains("auth[username]"), "login.php não gerou o formulário");
        let cookie = neo
            .lines()
            .find(|l| l.to_lowercase().starts_with("set-cookie:"))
            .and_then(|l| l.split(':').nth(1))
            .and_then(|c| c.split(';').next())
            .unwrap_or("")
            .trim()
            .to_string();
        println!("cookie de sessão: {cookie}");
        let form = format!(
            "auth%5Bdriver%5D=mysql&auth%5Bserver%5D=127.0.0.1%3A{db}&auth%5Busername%5D=root&auth%5Bpassword%5D=root&auth%5Bdb%5D=extras"
        );
        let post = http_request(
            admin,
            "POST",
            "/adminneo/index.php",
            &format!("Cookie: {cookie}\r\nContent-Type: application/x-www-form-urlencoded\r\n"),
            &form,
        );
        println!("--- login POST: {}", post.lines().next().unwrap_or(""));
        let location = post.lines().find(|l| l.to_lowercase().starts_with("location:")).unwrap_or("(sem Location)");
        println!("{location}");
        // Segue o redirecionamento com o mesmo cookie
        let target = location.splitn(2, ':').nth(1).unwrap_or("/adminneo/").trim().to_string();
        let target = if target.starts_with('/') { target } else { format!("/adminneo/{target}") };
        // O login regenera a sessão: o navegador passa a usar o cookie novo.
        let new_cookie = post
            .lines()
            .find(|l| l.to_lowercase().starts_with("set-cookie:"))
            .and_then(|l| l.split(':').nth(1))
            .and_then(|c| c.split(';').next())
            .unwrap_or(&cookie)
            .trim()
            .to_string();
        let after = http_request(
            admin,
            "GET",
            &target,
            &format!("Cookie: {new_cookie}\r\nAccept-Language: pt-BR\r\n"),
            "",
        );
        println!("--- headers do POST:\n{}", post.split("\r\n\r\n").next().unwrap_or(""));
        let text: String = after.split("\r\n\r\n").nth(1).unwrap_or("").split('<').filter_map(|c| c.split('>').nth(1)).map(|t| t.trim()).filter(|t| !t.is_empty()).take(25).collect::<Vec<_>>().join(" | ");
        println!("--- texto da página após login: {text}");
        println!("--- depois do login: {} | pede senha: {} | tem logout: {}", after.lines().next().unwrap_or(""), after.contains("auth[password]"), after.to_lowercase().contains("logout"));
        assert!(!after.contains("auth[password]"), "Adminer Neo continuou pedindo login");

        // Caminhos fora de /adminneo não podem ser servidos
        let outside = http_get_path(admin, "/mailpit/mailpit");
        assert!(outside.starts_with("HTTP/1.0 404"), "o site admin expôs arquivos indevidos: {}", outside.lines().next().unwrap());

        // mail() do PHP -> Mailpit
        let docroot = base.join("projects/extras/app");
        fs::write(
            docroot.join("mail.php"),
            "<?php var_dump(mail('dest@example.test', 'Assunto de teste', 'Corpo da mensagem', 'From: app@localtusk.test'));",
        )
        .unwrap();
        let out = http_get_path(http, "/mail.php");
        println!("--- mail.php: {}", out.lines().last().unwrap_or(""));
        std::thread::sleep(Duration::from_millis(800));
        let api = http_get_path(mail_ui, "/api/v1/messages");
        let body = api.split("\r\n\r\n").nth(1).unwrap_or("");
        println!("--- Mailpit API: {}", &body[..body.len().min(300)]);
        assert!(body.contains("Assunto de teste"), "o e-mail não chegou ao Mailpit");

        stop(&mut running);
        assert!(project::port_is_free(admin) && project::port_is_free(mail_ui));
        let _ = fs::remove_dir_all(&base);
    }

    fn dir_size(p: &Path) -> u64 {
        fs::read_dir(p)
            .map(|rd| {
                rd.flatten()
                    .map(|e| match e.metadata() {
                        Ok(m) if m.is_dir() => dir_size(&e.path()),
                        Ok(m) => m.len(),
                        Err(_) => 0,
                    })
                    .sum()
            })
            .unwrap_or(0)
    }

    /// Ambiente 100% gerenciado: instala Caddy, MySQL 8.4 e PHP 8.3 numa pasta limpa e sobe um
    /// projeto com eles (versões diferentes das do Homebrew, para provar que não são as do sistema).
    /// Baixa ~230 MB. Rode com: `cargo test --lib managed_stack -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn managed_stack() {
        let base = std::env::temp_dir().join(format!("localtusk-test-stack-{}", project::now()));
        let t = Instant::now();

        let cat = mysql_manager::catalog(&base).expect("catálogo do MySQL");
        println!("catálogo do MySQL em {:?}: {} versões", t.elapsed(), cat.len());
        assert!(t.elapsed() < Duration::from_millis(200), "o catálogo deveria ser instantâneo (embutido)");
        for series in ["8.0", "8.4", "9.7"] {
            let v: Vec<&str> = cat.iter().filter(|r| r.series == series).map(|r| r.version.as_str()).collect();
            println!("  {series}: {} versões, mais nova {:?}", v.len(), v.first());
        }
        let my = cat.iter().find(|r| r.series == "8.4" && r.latest).expect("8.4").version.clone();
        let pcat = crate::php_manager::catalog(&base).expect("catálogo do PHP");
        let php = pcat.iter().find(|r| r.minor == "8.3" && r.latest).expect("php 8.3").version.clone();

        let show = |p: InstallProgress| {
            if p.stage != "download" {
                println!("  [{}] {}", p.id, p.stage);
            }
        };
        extras::install(&base, "caddy", &show).expect("caddy");
        println!("Caddy {:?}", extras::status(&base).iter().find(|s| s.id == "caddy").unwrap().version);
        mysql_manager::install(&base, &my, &show).expect("mysql");
        println!("MySQL {my} instalado: {} MB em disco", dir_size(&mysql_manager::install_dir(&base, &my)) / 1024 / 1024);
        crate::php_manager::install(&base, &php, &show).expect("php");

        let http = project::free_port(18300, &Default::default()).unwrap();
        let db = project::free_port(13600, &Default::default()).unwrap();
        let p = Project {
            id: "stack".into(),
            name: "Stack".into(),
            docroot: "".into(),
            php_version: "8.3".into(),
            db_version: "8.4".into(),
            http_port: http,
            db_port: db,
            db_name: "stack".into(),
            created_at: 0,
            ..Default::default()
        };
        let mut running = start(&base, &p).expect("start falhou");
        let body = http_get(http);
        println!("{}", body.lines().filter(|l| l.contains("X-Powered") || l.contains("Banco")).collect::<Vec<_>>().join("\n"));
        assert!(body.contains(&format!("X-Powered-By: PHP/{php}")), "não usou o PHP gerenciado");
        assert!(body.contains(&format!("MySQL {my}")), "não usou o MySQL gerenciado");
        stop(&mut running);
        assert!(project::port_is_free(http) && project::port_is_free(db));

        // Segunda subida: reaproveita o banco já criado
        let mut running = start(&base, &p).expect("2º start falhou");
        assert!(http_get(http).contains(&format!("MySQL {my}")));
        stop(&mut running);

        assert!(db_initialized(&base, "stack"));
        let _ = fs::remove_dir_all(&base);
    }

    /// Instala o WordPress de verdade (WP-CLI + PHP CLI + MySQL), confere o site e os atalhos
    /// do terminal. Baixa ~250 MB na primeira vez (guarda em /tmp/localtusk-test-wp para as próximas).
    /// Rode com: `cargo test --lib wordpress_end_to_end -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn wordpress_end_to_end() {
        let base = std::env::temp_dir().join("localtusk-test-wp");
        let _ = fs::remove_dir_all(project::project_dir(&base, "wp"));
        let show = |p: InstallProgress| {
            if p.stage != "download" {
                println!("  [{}] {} {}", p.id, p.stage, p.message.unwrap_or_default());
            }
        };

        if !extras::is_installed(&base, "caddy") {
            extras::install(&base, "caddy", &show).expect("caddy");
        }
        if mysql_manager::pick(&base, "8.4").is_none() {
            let cat = mysql_manager::catalog(&base).expect("catálogo");
            let my = cat.iter().find(|r| r.series == "8.4" && r.latest).unwrap().version.clone();
            mysql_manager::install(&base, &my, &show).expect("mysql");
        }
        if crate::tools::pick_php(&base, "8.3").map(|i| !i.managed).unwrap_or(true) {
            let cat = crate::php_manager::catalog(&base).expect("catálogo do PHP");
            let php = cat.iter().find(|r| r.minor == "8.3" && r.latest).unwrap().version.clone();
            crate::php_manager::install(&base, &php, &show).expect("php");
        }

        let http = project::free_port(18400, &Default::default()).unwrap();
        let db = project::free_port(13700, &Default::default()).unwrap();
        let p = Project {
            id: "wp".into(),
            name: "Meu WP".into(),
            docroot: "".into(),
            php_version: "8.3".into(),
            db_version: "8.4".into(),
            http_port: http,
            db_port: db,
            db_name: "wp".into(),
            created_at: 0,
            ..Default::default()
        };
        scaffold(&base, &p).unwrap();
        let mut running = start(&base, &p).expect("start falhou");

        // Pasta com arquivos do usuário: só é apagada com a confirmação (replace_existing).
        fs::write(docroot_dir(&base, &p).join("lixo.txt"), "x").unwrap();
        let mut opts = crate::wp::WpOptions {
            title: "Blog de Teste".into(),
            admin_user: "admin".into(),
            admin_password: "SenhaForte123".into(),
            admin_email: "admin@example.com".into(),
            locale: "pt_BR".into(),
            replace_existing: false,
        };
        assert!(crate::wp::install_wordpress(&base, &p, &opts, &show).is_err(), "deveria recusar sem confirmação");
        assert!(docroot_dir(&base, &p).join("lixo.txt").exists());
        opts.replace_existing = true;
        let t = Instant::now();
        crate::wp::install_wordpress(&base, &p, &opts, &show).expect("install_wordpress");
        println!("WordPress instalado em {:?}", t.elapsed());
        assert!(crate::wp::is_wordpress(&base, &p));
        assert!(!docroot_dir(&base, &p).join("lixo.txt").exists(), "o arquivo antigo deveria ter sido apagado");

        // Instalar de novo é recusado.
        assert!(crate::wp::install_wordpress(&base, &p, &opts, &show).is_err());

        // O site é instalado e acessado pelo domínio do projeto (<id>.localhost).
        let host = format!("Host: {}:{http}\r\n", project::site_host(&p));
        let home = http_request(http, "GET", "/", &host, "");
        assert!(home.contains("200 OK"), "home: {}", home.lines().take(5).collect::<Vec<_>>().join("\n"));
        assert!(home.contains("Blog de Teste"), "título não aparece na home");
        let login = http_request(http, "GET", "/wp-login.php", &host, "");
        assert!(login.contains("200 OK") && login.contains("user_login"), "wp-login não abriu");
        // Links permanentes bonitos: /wp-json/ passa pelo index.php.
        let json = http_request(http, "GET", "/wp-json/", &host, "");
        assert!(json.contains("200 OK") && json.contains("Blog de Teste"), "REST API/permalinks não funcionam");
        // Login de verdade.
        let cookie = "Cookie: wordpress_test_cookie=WP%20Cookie%20check\r\n";
        let post = http_request(
            http,
            "POST",
            "/wp-login.php",
            &format!("{host}{cookie}Content-Type: application/x-www-form-urlencoded\r\n"),
            "log=admin&pwd=SenhaForte123&wp-submit=Log+In",
        );
        assert!(post.contains("302") && post.contains("wordpress_logged_in"), "login do admin falhou: {}", post.lines().take(12).collect::<Vec<_>>().join("\n"));

        // Atalhos do terminal.
        let script = crate::wp::prepare_terminal(&base, &p).expect("terminal");
        let shims = script.parent().unwrap().join("shims");
        let sh = |cmd: &str| {
            let out = Command::new("/bin/sh")
                .arg("-c")
                .arg(format!("cd '{}' && {cmd}", runtime_docroot(&base, &p)))
                .env("PATH", format!("{}:/usr/bin:/bin", shims.display()))
                .output()
                .unwrap();
            let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
            println!("$ {cmd}\n{}", text.trim());
            assert!(out.status.success(), "{cmd} falhou");
            text
        };
        assert!(sh("php -v").contains("PHP 8.3"));
        assert!(sh("wp core version").trim().chars().next().unwrap().is_ascii_digit());
        assert!(sh("wp option get blogname").contains("Blog de Teste"));
        assert!(sh("mysql -e 'select count(*) as posts from wp_posts'").contains("posts"));

        // Backup e restauração: o WordPress volta em OUTRO projeto (outro id, porta e banco).
        let archive = crate::backup::create(&base, &p, "", "bk", &show).expect("backup");
        let manifest = crate::backup::read_manifest(&archive).expect("manifest");
        assert!(manifest.id == "wp" && manifest.has_wordpress);
        assert_eq!(crate::backup::list(&base, Some("wp")).len(), 1);
        assert!(crate::backup::list(&base, Some("outro")).is_empty());
        println!("backup: {} ({} bytes)", archive.display(), fs::metadata(&archive).unwrap().len());

        let http2 = project::free_port(18500, &Default::default()).unwrap();
        let db2 = project::free_port(13800, &Default::default()).unwrap();
        let p2 = Project {
            id: "wp2".into(),
            name: "Meu WP 2".into(),
            docroot: "".into(),
            php_version: "8.3".into(),
            db_version: "8.4".into(),
            http_port: http2,
            db_port: db2,
            db_name: "wp2".into(),
            created_at: 0,
            ..Default::default()
        };
        scaffold(&base, &p2).unwrap();
        let mut running2 = start(&base, &p2).expect("start do projeto restaurado");
        let warning = crate::backup::restore_into(&base, &p2, &archive, "bk", &show).expect("restore");
        assert!(warning.is_none(), "aviso inesperado: {warning:?}");

        let host2 = format!("Host: {}:{http2}\r\n", project::site_host(&p2));
        let home2 = http_request(http2, "GET", "/", &host2, "");
        assert!(home2.contains("200 OK") && home2.contains("Blog de Teste"), "site restaurado não abriu: {}", home2.lines().take(6).collect::<Vec<_>>().join("\n"));
        let post2 = http_request(
            http2,
            "POST",
            "/wp-login.php",
            &format!("{host2}{cookie}Content-Type: application/x-www-form-urlencoded\r\n"),
            "log=admin&pwd=SenhaForte123&wp-submit=Log+In",
        );
        assert!(post2.contains("302") && post2.contains("wordpress_logged_in"), "login no projeto restaurado falhou");
        let cfg = fs::read_to_string(docroot_dir(&base, &p2).join("wp-config.php")).unwrap();
        assert!(cfg.contains("'wp2'") && cfg.contains(&format!("127.0.0.1:{db2}")), "wp-config não aponta para o banco novo");
        stop(&mut running2);
        let _ = fs::remove_dir_all(project::project_dir(&base, "wp2"));

        stop(&mut running);
        let _ = fs::remove_dir_all(project::project_dir(&base, "wp"));
    }

    /// Blueprint de verdade: um projeto com arquivo e dados vira modelo, e um segundo projeto
    /// (outro id e outras portas) nasce com os mesmos arquivos e o mesmo banco.
    /// Usa o PHP, o MySQL e o Caddy do sistema. Rode com:
    /// `cargo test --lib blueprint_end_to_end -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn blueprint_end_to_end() {
        let base = std::env::temp_dir().join(format!("localtusk-test-bp-{}", project::now()));
        let mk = |id: &str, http_start: u16, db_start: u16| {
            let http = project::free_port(http_start, &Default::default()).unwrap();
            let db = project::free_port(db_start, &Default::default()).unwrap();
            Project {
                id: id.into(),
                name: id.into(),
                docroot: "public".into(),
                php_version: "".into(),
                db_version: "".into(),
                http_port: http,
                db_port: db,
                db_name: "modelo".into(),
                created_at: 0,
                ..Default::default()
            }
        };
        let show = |_: InstallProgress| {};
        let page = r#"<?php
$pdo = new PDO('mysql:host='.getenv('DB_HOST').';port='.getenv('DB_PORT').';dbname='.getenv('DB_NAME'), getenv('DB_USER'), getenv('DB_PASSWORD'));
$pdo->exec('CREATE TABLE IF NOT EXISTS notas (id INT PRIMARY KEY AUTO_INCREMENT, texto VARCHAR(50))');
if (isset($_GET['grava'])) { $pdo->exec("INSERT INTO notas (texto) VALUES ('semente')"); }
echo 'notas=' . $pdo->query('SELECT COUNT(*) FROM notas')->fetchColumn();
"#;

        let a = mk("origem", 18600, 13900);
        scaffold(&base, &a).unwrap();
        fs::create_dir_all(docroot_dir(&base, &a)).unwrap();
        fs::write(docroot_dir(&base, &a).join("index.php"), page).unwrap();
        let mut run_a = start(&base, &a).expect("start origem");
        assert!(http_get_path(a.http_port, "/?grava=1").contains("notas=1"));

        let path = crate::blueprint::create(&base, &a, "Modelo de teste", "descrição", "bp", &show).expect("blueprint");
        let info = crate::blueprint::info(&path).unwrap();
        assert_eq!((info.name.as_str(), info.source_name.as_str(), info.docroot.as_str()), ("Modelo de teste", "origem", "public"));
        assert_eq!(crate::blueprint::list(&base).len(), 1);
        // O blueprint não aparece na lista de backups.
        assert!(crate::backup::list(&base, None).is_empty());
        stop(&mut run_a);

        let b = mk("copia", 18700, 14000);
        scaffold(&base, &b).unwrap();
        let mut run_b = start(&base, &b).expect("start cópia");
        let warning = crate::backup::restore_into(&base, &b, &path, "bp", &show).expect("restore");
        assert!(warning.is_none(), "aviso inesperado: {warning:?}");
        let body = http_get_path(b.http_port, "/");
        assert!(body.contains("notas=1"), "o novo projeto não herdou os dados: {body}");
        stop(&mut run_b);

        crate::blueprint::delete(&base, &path).unwrap();
        assert!(crate::blueprint::list(&base).is_empty());
        let _ = fs::remove_dir_all(&base);
    }

    fn runtime_docroot(base: &Path, p: &Project) -> String {
        docroot_dir(base, p).display().to_string()
    }

    fn caddy_sample() -> Project {
        Project { id: "loja".into(), http_port: 8000, db_port: 3307, ..Default::default() }
    }

    fn render(p: &Project, lan_hosts: &[String]) -> String {
        render_caddyfile(p, "/p/app", "/p/logs", 9000, None, lan_hosts)
    }

    #[test]
    fn caddyfile_padrao_e_so_http_local() {
        let c = render(&caddy_sample(), &["192.168.0.10".into()]);
        assert!(c.contains("auto_https off"));
        assert!(c.contains(":8000 {\n\tbind 127.0.0.1\n\timport app"));
        assert!(!c.contains("tls internal") && !c.contains("0.0.0.0") && !c.contains("192.168"));
    }

    #[test]
    fn caddyfile_https_usa_a_ca_local_e_nao_redireciona() {
        let mut p = caddy_sample();
        p.https = true;
        p.https_port = 8443;
        let c = render(&p, &["192.168.0.10".into()]);
        assert!(c.contains("auto_https disable_redirects") && c.contains("skip_install_trust"));
        assert!(!c.contains("auto_https off"));
        assert!(c.contains("https://loja.localhost:8443, https://127.0.0.1:8443 {\n\tbind 127.0.0.1\n\ttls internal"));
        assert!(!c.contains("192.168"), "o IP da rede só entra com a rede ligada");
        // O HTTP continua disponível.
        assert!(c.contains(":8000 {"));
    }

    #[test]
    fn caddyfile_rede_escuta_em_todas_as_interfaces_e_inclui_os_nomes_no_certificado() {
        let mut p = caddy_sample();
        p.lan = true;
        p.https = true;
        p.https_port = 8443;
        let c = render(&p, &["192.168.0.10".into(), "meu-mac.local".into()]);
        assert_eq!(c.matches("bind 0.0.0.0").count(), 2, "{c}");
        assert!(!c.contains("bind 127.0.0.1"));
        assert!(c.contains("https://192.168.0.10:8443") && c.contains("https://meu-mac.local:8443"));
    }

    #[test]
    fn caddyfile_rede_sem_https_nao_cria_site_tls() {
        let mut p = caddy_sample();
        p.lan = true;
        let c = render(&p, &["192.168.0.10".into()]);
        assert!(c.contains(":8000 {\n\tbind 0.0.0.0"));
        assert!(!c.contains("tls internal"));
    }

    #[test]
    fn adminer_continua_so_local_mesmo_com_a_rede_ligada() {
        let mut p = caddy_sample();
        p.lan = true;
        let c = render_caddyfile(&p, "/p/app", "/p/logs", 9000, Some((9100, "/tools".into())), &[]);
        let admin = &c[c.find(":9100 {").unwrap()..];
        assert!(admin.starts_with(":9100 {\n\tbind 127.0.0.1"), "{admin}");
    }

    /// HTTPS de verdade: o Caddy emite o certificado da CA local, o PHP enxerga `HTTPS=on` e o
    /// certificado vale para `<id>.localhost` (verificado com o certificado raiz, sem confiar no sistema).
    /// Usa PHP, MySQL e Caddy já instalados. Rode com:
    /// `cargo test --lib https_end_to_end -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn https_end_to_end() {
        let base = std::env::temp_dir().join(format!("localtusk-test-https-{}", project::now()));
        let http = project::free_port(18400, &Default::default()).unwrap();
        let https = project::free_port(18450, &Default::default()).unwrap();
        let db = project::free_port(13700, &Default::default()).unwrap();
        let p = Project {
            id: "seguro".into(),
            name: "Seguro".into(),
            http_port: http,
            https_port: https,
            https: true,
            db_port: db,
            db_name: "seguro".into(),
            ..Default::default()
        };
        scaffold(&base, &p).unwrap();
        fs::write(docroot_dir(&base, &p).join("index.php"), "<?php echo 'https=' . ($_SERVER['HTTPS'] ?? 'off');").unwrap();
        let caddy = extras::caddy_bin(&dirs::home_dir().unwrap().join("LocalTusk")).expect("Caddy instalado");
        // Reaproveita o Caddy já baixado pelo app: copia para a pasta de teste.
        let dest = extras::caddy_bin(&base);
        if dest.is_none() {
            let target = base.join("bin").join("tools").join("caddy");
            fs::create_dir_all(&target).unwrap();
            fs::copy(&caddy, target.join(caddy.file_name().unwrap())).unwrap();
        }
        let mut running = start(&base, &p).expect("start falhou");

        let root = crate::ssl::root_cert(&base);
        assert!(root.is_file(), "a CA deveria ter sido criada em {}", root.display());
        let url = format!("https://{}:{https}/", project::site_host(&p));
        let out = Command::new("curl")
            .args(["-s", "--max-time", "10", "--cacert"])
            .arg(&root)
            .arg(&url)
            .output()
            .unwrap();
        assert!(out.status.success(), "curl: {} / {}", out.status, String::from_utf8_lossy(&out.stderr));
        assert_eq!(String::from_utf8_lossy(&out.stdout), "https=on");
        // Sem a CA, o certificado não é confiável (comprova que não é um site HTTP disfarçado).
        let bad = Command::new("curl").args(["-s", "--max-time", "10", "--cacert", "/dev/null", &url]).status().unwrap();
        assert!(!bad.success());
        // O HTTP continua respondendo.
        assert!(http_get(http).contains("https=off"));
        // E o status da CA sabe que ela existe.
        assert!(crate::ssl::status(&base).ca_ready);

        stop(&mut running);
        assert!(project::port_is_free(https), "Caddy ainda no ar");
        let _ = fs::remove_dir_all(&base);
    }

    /// Acesso pela rede: desligado, o IP da rede recusa a conexão; ligado, responde por HTTP e
    /// por HTTPS (certificado válido para o IP). Rode com:
    /// `cargo test --lib lan_end_to_end -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn lan_end_to_end() {
        let Some(ip) = project::lan_ip() else {
            println!("sem rede local: teste ignorado");
            return;
        };
        let base = std::env::temp_dir().join(format!("localtusk-test-lan-{}", project::now()));
        let http = project::free_port(18500, &Default::default()).unwrap();
        let https = project::free_port(18550, &Default::default()).unwrap();
        let db = project::free_port(13800, &Default::default()).unwrap();
        let mut p = Project {
            id: "rede".into(),
            name: "Rede".into(),
            http_port: http,
            https_port: https,
            https: true,
            db_port: db,
            db_name: "rede".into(),
            ..Default::default()
        };
        scaffold(&base, &p).unwrap();
        fs::write(docroot_dir(&base, &p).join("index.php"), "<?php echo 'ola ' . $_SERVER['HTTP_HOST'];").unwrap();
        let caddy = extras::caddy_bin(&dirs::home_dir().unwrap().join("LocalTusk")).expect("Caddy instalado");
        let target = base.join("bin").join("tools").join("caddy");
        fs::create_dir_all(&target).unwrap();
        fs::copy(&caddy, target.join(caddy.file_name().unwrap())).unwrap();

        let curl = |extra: &[&str], url: &str| {
            Command::new("curl").args(["-s", "--max-time", "10"]).args(extra).arg(url).output().unwrap()
        };

        // 1) Só local: o IP da rede recusa.
        let mut running = start(&base, &p).expect("start falhou");
        let blocked = curl(&[], &format!("http://{ip}:{http}/"));
        assert!(!blocked.status.success(), "sem a rede ligada o IP da rede deveria recusar");
        assert!(http_get(http).contains("ola"), "o acesso local deveria funcionar");
        stop(&mut running);

        // 2) Rede ligada.
        p.lan = true;
        let mut running = start(&base, &p).expect("start falhou");
        let ok = curl(&[], &format!("http://{ip}:{http}/"));
        assert_eq!(String::from_utf8_lossy(&ok.stdout), format!("ola {ip}:{http}"));
        let root = crate::ssl::root_cert(&base);
        let secure = curl(&["--cacert", root.to_str().unwrap()], &format!("https://{ip}:{https}/"));
        assert!(secure.status.success(), "HTTPS pelo IP: {}", String::from_utf8_lossy(&secure.stderr));
        assert_eq!(String::from_utf8_lossy(&secure.stdout), format!("ola {ip}:{https}"));
        // O banco e o Adminer continuam só locais.
        let blocked_db = std::net::TcpStream::connect_timeout(&SocketAddr::from((ip, db)), Duration::from_secs(2));
        assert!(blocked_db.is_err(), "o MySQL não pode ficar exposto na rede");
        stop(&mut running);
        let _ = fs::remove_dir_all(&base);
    }
}
