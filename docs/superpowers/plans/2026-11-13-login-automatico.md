# Login automático no wp-admin — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** O botão wp-admin de um projeto WordPress entra já logado como o administrador escolhido, com chave liga/desliga e seletor de usuário por projeto.

**Architecture:** Um mu-plugin (`localtusk-login.php`) aceita `?localtusk-login=<código>`; o app gera um código aleatório de uso único, grava o hash dele como transient (60 s) via wp-cli e abre a URL. A configuração fica no `Project` (`auto_login`, `auto_login_user`); três comandos Tauri ligam/desligam, listam admins e abrem o wp-admin.

**Tech Stack:** Rust (Tauri 2, `sha2` já no Cargo), WP-CLI, PHP (mu-plugin), React/TS.

**Spec:** `docs/superpowers/specs/2026-11-13-login-automatico-design.md`

## Global Constraints

- Parâmetro da URL: `localtusk-login`. Arquivo do mu-plugin: `localtusk-login.php`.
- Transient: nome `localtusk_login_<sha256 hex do código>`, valor = `user_login`, validade `60` s. O código em si nunca é gravado.
- Código: 32 bytes aleatórios de `/dev/urandom`, em hex (64 caracteres). Falha ao ler = erro (sem fallback inseguro).
- Login só para usuário que ainda é administrador (`manage_options`); uso único (apagar o transient antes de logar).
- Código inválido, vencido ou já usado: redireciona para `wp_login_url()`, sem mensagem técnica.
- Padrão: `auto_login = false`. Projetos antigos sem os campos carregam normalmente (`#[serde(default)]`).
- Textos exatos: "Login automático no wp-admin"; "Usuário"; "O usuário <nome> não é mais administrador deste site. Escolha outro no login automático."
- Nunca escrever fora do projeto do LocalTusk. Sem novas dependências no Cargo nem no package.json.
- Commits em português, estilo `feat(login-automatico): …`, terminando com `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Sem push.

## Review Focus

1. Código reaproveitado (segundo clique na mesma URL, histórico do navegador) → tela de login, sem logar. Teste em Task 1.
2. Código vencido (> 60 s) → tela de login. Teste em Task 1 (transient com validade 1 s).
3. Usuário salvo apagado ou rebaixado → abre a tela de login normal e mostra a mensagem exata. Teste em Task 2 (`resolve_admin`).
4. Desligar com o projeto parado → o mu-plugin é removido mesmo sem banco (caminho padrão). Teste em Task 1.
5. `projects.json` antigo sem os campos novos → carrega com login desligado. Teste em Task 2.

---

### Task 1: Núcleo no `wp.rs` — mu-plugin, código e admins

**Files:**
- Modify: `src-tauri/src/wp.rs`
- Test: `src-tauri/src/wp.rs` (unidade) e `src-tauri/src/runtime.rs` (e2e `#[ignore]`)

**Interfaces:**
- Produces (todas `pub` em `wp.rs`):
  - `pub const LOGIN_PARAM: &str = "localtusk-login";`
  - `pub fn admin_users(base: &Path, p: &Project) -> Result<Vec<String>, String>` — `wp user list --role=administrator --field=user_login`, uma por linha, na ordem do wp-cli.
  - `pub fn install_login_plugin(base: &Path, p: &Project) -> Result<(), String>` — grava o mu-plugin (sobrescreve; idempotente).
  - `pub fn remove_login_plugin(base: &Path, p: &Project) -> Result<(), String>` — apaga se existir; funciona com o projeto parado.
  - `pub fn create_login_token(base: &Path, p: &Project, user: &str) -> Result<String, String>` — gera o código, grava o transient (`wp transient set <nome> <user> 60`), devolve o código.
  - `fn login_token_key(token: &str) -> String` (privada) — `localtusk_login_` + sha256 hex.

- [ ] **Step 1: Testes de unidade que falham**

```rust
#[test]
fn login_token_key_usa_hash() {
    let k = login_token_key("abc");
    assert_eq!(k, "localtusk_login_ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
}
#[test]
fn codigo_aleatorio_tem_64_hex_e_muda() {
    let a = random_token().unwrap();
    let b = random_token().unwrap();
    assert_eq!(a.len(), 64);
    assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    assert_ne!(a, b);
}
#[test]
fn plugin_confere_usuario_e_uso_unico() {
    // O PHP do mu-plugin tem as peças que garantem a segurança.
    for s in ["localtusk-login", "localtusk_login_", "hash('sha256'", "delete_transient", "manage_options", "wp_set_auth_cookie", "wp_login_url()"] {
        assert!(LOGIN_PLUGIN_PHP.contains(s), "falta {s}");
    }
}
```

Run: `cd src-tauri && cargo test --lib wp::tests` → FAIL (não compila).

- [ ] **Step 2: Implementar**

- `fn random_token() -> Result<String, String>`: lê 32 bytes de `/dev/urandom`.
- `const LOGIN_PLUGIN_PHP: &str`: mu-plugin que, em `init` com prioridade 1, se `$_GET['localtusk-login']` existir: calcula a chave, lê e **apaga** o transient, busca o usuário por login e confere `user_can($u, 'manage_options')`; se tudo bem, `wp_set_current_user`, `wp_set_auth_cookie($u->ID, false, is_ssl())`, `wp_safe_redirect(admin_url())`; senão `wp_safe_redirect(wp_login_url())`; `exit` nos dois casos.
- Pasta do mu-plugin: `wp eval 'echo WPMU_PLUGIN_DIR;'`; se falhar (projeto parado), `<docroot>/wp-content/mu-plugins`. Criar a pasta se faltar.
- Para ler a saída do wp-cli, criar `fn wp_output(base, p, args: &[&str], timeout: Duration) -> Result<String, String>` reaproveitando `php_cli`, `wp_command` e o log de `run_logged` para stderr.

- [ ] **Step 3: Rodar unidade** — `cargo test --lib wp::tests` → PASS.

- [ ] **Step 4: e2e `auto_login_end_to_end` (`#[ignore]`, em `runtime.rs`)**, com o mesmo preparo de `wordpress_end_to_end` (pode extrair um helper comum; não mudar o que aquele teste verifica). Asserções:
  - `admin_users` devolve `["admin"]` (o usuário criado na instalação).
  - Depois de `install_login_plugin` e `create_login_token(…, "admin")`: `GET /?localtusk-login=<código>` responde `302`, `Location` termina em `/wp-admin/` e há `Set-Cookie: wordpress_logged_in_`.
  - `GET /wp-admin/` com esses cookies responde `200` e contém `id="wpadminbar"`.
  - O mesmo código de novo: `302` para `wp-login.php` e **sem** `wordpress_logged_in_`.
  - Código vencido: grava um transient com validade 1 s (via wp-cli no teste), espera 2 s, usa → `wp-login.php`, sem cookie.
  - Código para usuário que não é admin (`wp user create leitor l@x.test --role=subscriber`) → `wp-login.php`, sem cookie.
  - Para o projeto e chama `remove_login_plugin` → o arquivo não existe mais.
  - Nenhum processo do teste (mysqld, php-fpm, caddy) fica rodando.

Run: `cargo test --lib auto_login_end_to_end -- --ignored` → PASS; e `wordpress_end_to_end -- --ignored` continua passando (apague `/tmp/localtusk-test-wp/backups/wp-*.tar.gz` antes, o teste não limpa os próprios backups).

- [ ] **Step 5: Commit** — `feat(login-automatico): mu-plugin e código de uso único`

---

### Task 2: Configuração no projeto e comandos Tauri

**Files:**
- Modify: `src-tauri/src/project.rs` (campos), `src-tauri/src/commands.rs` (view + comandos), `src-tauri/src/lib.rs` (registro)

**Interfaces:**
- Consumes: Task 1 (`admin_users`, `install_login_plugin`, `remove_login_plugin`, `create_login_token`, `LOGIN_PARAM`).
- Produces:
  - `Project { #[serde(default)] pub auto_login: bool, #[serde(default)] pub auto_login_user: String }`.
  - `ProjectView` ganha `auto_login: bool`, `auto_login_user: String` (camelCase no JSON: `autoLogin`, `autoLoginUser`).
  - `#[tauri::command] async fn set_auto_login(state, id: String, enabled: bool, user: String) -> Result<ProjectView, String>` — salva os dois campos (sem reiniciar o projeto). Ligado e projeto rodando com WordPress: `install_login_plugin`. Desligado: `remove_login_plugin` (erro aqui não impede salvar; ignora se não houver WordPress).
  - `#[tauri::command] async fn list_wp_admins(state, id: String) -> Result<Vec<String>, String>` — exige projeto rodando ("Inicie o projeto para listar os usuários.").
  - `#[tauri::command] async fn open_wp_admin(app, state, id: String) -> Result<Option<String>, String>` — `None` = abriu sem aviso; `Some(msg)` = abriu a tela de login normal e o app deve mostrar `msg`.
  - `fn resolve_admin(saved: &str, admins: &[String]) -> Result<String, String>` (pura) — devolve `saved` se estiver na lista; senão `Err` com a mensagem exata do Global Constraints (`<nome>` = `saved`).

- [ ] **Step 1: Testes de unidade que falham**

```rust
#[test]
fn projeto_antigo_vem_sem_login_automatico() {
    // JSON de um projeto salvo antes deste recurso (copie um existente dos testes de project.rs).
    let p: Project = serde_json::from_str(OLD_PROJECT_JSON).unwrap();
    assert!(!p.auto_login);
    assert_eq!(p.auto_login_user, "");
}
#[test]
fn resolve_admin_aceita_quem_ainda_e_admin() {
    let admins = vec!["admin".to_string(), "ana".to_string()];
    assert_eq!(resolve_admin("ana", &admins).unwrap(), "ana");
    assert_eq!(
        resolve_admin("bia", &admins).unwrap_err(),
        "O usuário bia não é mais administrador deste site. Escolha outro no login automático."
    );
}
```

- [ ] **Step 2: Implementar.** `open_wp_admin`:
  - `auto_login` desligado, ou projeto sem WordPress → abre `/wp-admin/` como `open_site` faz; `Ok(None)`.
  - Ligado: `admin_users` → `resolve_admin`. Erro de `resolve_admin` → abre `/wp-admin/`, `Ok(Some(msg))`.
  - Senão `install_login_plugin` (garante o arquivo) → `create_login_token` → abre `format!("{}/?{}={token}", project::site_url(&p), LOGIN_PARAM)`; `Ok(None)`.
  - Qualquer erro de wp-cli → abre `/wp-admin/`, `Ok(Some(erro))`.
  - Rodar o trabalho de wp-cli fora da thread da UI (`blocking`, como os comandos vizinhos).

- [ ] **Step 3:** `cargo test --lib` → PASS; `cargo clippy --lib` sem avisos novos nos arquivos tocados.

- [ ] **Step 4: Commit** — `feat(login-automatico): configuração por projeto e comandos`

---

### Task 3: Interface

**Files:**
- Modify: `src/api.ts`, `src/WordPress.tsx`, `src/dev-mock.ts`

**Interfaces:**
- Consumes: Task 2 (comandos e campos `autoLogin`, `autoLoginUser`).
- Produces: `Project.autoLogin: boolean`, `Project.autoLoginUser: string`; `api.setAutoLogin(id, enabled, user) => Promise<Project>`, `api.listWpAdmins(id) => Promise<string[]>`, `api.openWpAdmin(id) => Promise<string | null>`.

- [ ] **Step 1:** Em `WordPressPanel`, com WordPress instalado, abaixo dos botões:

```
[x] Login automático no wp-admin   Usuário: [admin ▾]
```

  - O botão `wp-admin` passa a chamar `api.openWpAdmin`; resposta não nula → `props.onError(msg)`.
  - Com o projeto rodando, carrega `listWpAdmins` ao montar e quando `p.running` muda. Parado: o seletor mostra só o usuário salvo, desativado.
  - Ligar sem usuário salvo usa o primeiro da lista. Trocar o usuário chama `setAutoLogin(p.id, true, novo)`. Depois de cada `setAutoLogin`, `props.onChanged()`.
  - Lista vazia com o projeto rodando: a chave fica desativada.
  - Seguir o padrão visual do painel (classes já usadas em `WordPress.tsx`/`App.css`).
- [ ] **Step 2:** `dev-mock.ts`: os três comandos (admins `["admin", "editor-chefe"]`; `open_wp_admin` devolve `null`) e os campos novos nos projetos de exemplo.
- [ ] **Step 3:** `pnpm exec tsc --noEmit` e `pnpm build` → OK. Abrir o modo mock (`pnpm dev`) num navegador headless e salvar um print do painel WordPress com a chave ligada (caminho do print no relatório).
- [ ] **Step 4: Commit** — `feat(login-automatico): chave e seletor no painel do WordPress`
