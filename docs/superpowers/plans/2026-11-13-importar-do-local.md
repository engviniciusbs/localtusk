# Importar do Local — plano de implementação

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Listar os sites do Local instalados na máquina e importar um deles como projeto novo do LocalTusk (arquivos, banco, versões, URLs do WordPress ajustadas).

**Architecture:** Um módulo novo `local_import.rs` lê os dados do Local e valida os sites. A restauração (`backup::restore_into` + `commands::restore_impl`) passa a aceitar uma origem "pasta" além do `.tar.gz`, então o importador cria o projeto pelo mesmo caminho do blueprint, sem arquivo intermediário. A interface ganha a tela "Importar do Local", que pergunta sobre versões faltantes e reaproveita os comandos de instalação existentes (`install_php`/`install_mysql`).

**Tech Stack:** Rust (Tauri 2, serde_json), React + TypeScript, testes `cargo test`.

**Spec:** `docs/superpowers/specs/2026-11-13-importar-do-local-design.md`

## Global Constraints

- Nada é escrito, movido ou apagado dentro da pasta do Local nem na pasta de dados dele. Só leitura.
- Pasta de dados do Local: Linux `~/.config/Local`, macOS `~/Library/Application Support/Local`. Windows: não suportado (devolve "nenhuma instalação encontrada").
- Site com `site-statuses.json` = `running` não pode ser importado. Mensagem: "Pare o site no Local para importar (se o Local estiver fechado, abra-o e pare o site)."
- Sem `app/public`: "A pasta do site não foi encontrada: <caminho>." Sem `app/sql/local.sql`: "O Local ainda não salvou o banco deste site. Inicie e pare o site no Local e tente de novo."
- Dump desatualizado = algum arquivo em `<dados do Local>/run/<id>/mysql/data/` com mtime mais de 120 s depois do mtime do `local.sql`. Aviso: "O banco salvo pelo Local pode estar desatualizado. Inicie e pare o site no Local para garantir."
- Avisos que não bloqueiam: multisite → "Sites multisite podem precisar de ajustes manuais."; Apache → "Regras do .htaccess não são usadas pelo LocalTusk."
- Projeto importado: `docroot = "public"`, engine `mysql`, conteúdo de `app/public` em `<projeto>/app/public`.
- Links simbólicos: copia o conteúdo apontado, nunca o link; laços são pulados; pastas linkadas aparecem no aviso final.
- Versão: a mesma série (`8.2.29` → `8.2`; MySQL `8.4.0` → `8.4`) instalada é usada sem perguntar. Se faltar, a interface oferece **Instalar** (patch mais novo da série no catálogo) ou **Usar** (mais próxima instalada: mesma major primeiro, depois a mais nova); MySQL mais antigo que o do Local mostra "O banco pode não importar numa versão mais antiga."
- Textos da interface em português, no tom das telas existentes. Código novo isolado em arquivos próprios sempre que possível (fork).
- Backup e blueprint continuam com o comportamento atual; os testes existentes passam.

## Review Focus

1. Caminho do site com espaço e `~` (`~/Local Sites/snap`): deve ser expandido e funcionar. → teste em Task 1.
2. `sites.json` com um site cujo `path` não existe mais (o usuário apagou a pasta): o site aparece na lista com o bloqueio "pasta não encontrada", e o resto da lista carrega normalmente. → teste em Task 1.
3. `sites.json` malformado ou com campos faltando (`services` sem `php`): não derruba a lista; o site vem com versão vazia. → teste em Task 1.
4. Importação que falha no meio (dump com SQL inválido): o projeto criado é desfeito e a pasta do Local continua igual. → teste em Task 3.
5. Manifesto sintético com portas 0: o projeto ganha portas livres reais, nunca a porta 0. → teste em Task 3.

---

### Task 1: Ler os sites do Local (`local_import.rs`)

**Files:**
- Create: `src-tauri/src/local_import.rs`
- Modify: `src-tauri/src/lib.rs` (adicionar `mod local_import;`)
- Test: em `local_import.rs` (`#[cfg(test)] mod tests`), com fixtures montadas em pastas temporárias como em `blueprint.rs:185` (`temp_base`)

**Interfaces:**
- Produces:
  - `pub fn local_data_dir(home: &Path) -> Option<PathBuf>` (por `cfg(target_os)`; `None` no Windows)
  - `pub fn list_sites(data_dir: &Path, home: &Path) -> Result<Vec<LocalSite>, String>` — `Err` só se `sites.json` não existir/for ilegível; ordenado por nome
  - `pub fn find_site(data_dir: &Path, home: &Path, id: &str) -> Result<LocalSite, String>`
  - ```rust
    #[derive(Debug, Clone, Serialize)] #[serde(rename_all = "camelCase")]
    pub struct LocalSite {
        pub id: String, pub name: String, pub path: String, pub domain: String,
        pub php_version: String,   // completo do Local, ex. "8.2.29" ("" se ausente)
        pub mysql_version: String, // ex. "8.4.0"
        pub web_server: String,    // "nginx" | "apache" | ""
        pub multisite: bool,
        pub running: bool,
        pub dump_saved_at: u64,    // mtime do local.sql em s desde 1970; 0 se não existe
        pub dump_stale: bool,
        pub blocker: Option<String>,  // mensagem que impede a importação
        pub warnings: Vec<String>,    // multisite, apache, dump desatualizado
    }
    ```
  - `pub fn public_dir(site: &LocalSite) -> PathBuf` (`<path>/app/public`) e `pub fn dump_file(site: &LocalSite) -> PathBuf` (`<path>/app/sql/local.sql`)
  - `pub fn series(full: &str) -> String` (`"8.2.29"` → `"8.2"`)

- [ ] **Step 1: Escrever os testes que falham**

Fixture helper `fake_local(dir) -> (data_dir, home)` que grava um `sites.json` e um `site-statuses.json` com os sites abaixo e cria as pastas correspondentes:

```rust
#[test] fn lista_site_parado_pronto() {
    // "snap": path "~/Local Sites/snap", php 8.2.29, mysql 8.4.0, nginx, halted, com app/public e app/sql/local.sql
    let s = &list_sites(&data, &home).unwrap()[..];
    let snap = s.iter().find(|x| x.id == "snap1").unwrap();
    assert_eq!(snap.path, home.join("Local Sites/snap").display().to_string());
    assert_eq!((snap.php_version.as_str(), snap.mysql_version.as_str(), snap.web_server.as_str()), ("8.2.29", "8.4.0", "nginx"));
    assert!(snap.blocker.is_none() && !snap.running && snap.dump_saved_at > 0);
}
#[test] fn site_rodando_fica_bloqueado()      // status "running" → blocker == Some(msg de "Pare o site no Local…")
#[test] fn site_sem_dump_fica_bloqueado()     // sem local.sql → blocker com "ainda não salvou o banco"
#[test] fn site_sem_pasta_fica_bloqueado_e_lista_continua() // path inexistente → blocker "pasta do site não foi encontrada"; os outros sites seguem na lista
#[test] fn campos_faltando_nao_derrubam()     // entrada sem "services" → php_version == "" e mysql_version == ""
#[test] fn multisite_e_apache_avisam()        // "multiSite": "ms-subdir" e services.apache → warnings contém as duas mensagens; blocker None
#[test] fn dump_desatualizado_avisa()         // run/<id>/mysql/data/ibdata1 com mtime = dump + 300 s → dump_stale && warnings contém a msg; com + 1 s → !dump_stale
#[test] fn sem_local_instalado_da_erro()      // data_dir sem sites.json → Err
#[test] fn serie_da_versao()                  // series("8.2.29")=="8.2", series("8.4")=="8.4", series("")==""
```

Usar `filetime`? Não está nas dependências; ajustar mtime com `std::fs::File::set_modified` (estável no Rust 1.75+).

- [ ] **Step 2: Rodar e ver falhar**

Run: `cd src-tauri && cargo test --lib local_import`
Expected: erro de compilação (módulo/funções não existem).

- [ ] **Step 3: Implementar**

Ler os JSON com `serde_json::Value` (tolerante a campos ausentes). `path` com `~` no início é expandido com `home`. `multisite` = `multiSite` é string não vazia e diferente de `"no"`. `web_server` = `"apache"` se `services` tem a chave `apache`, senão `"nginx"` se tem `nginx`, senão `""`. `dump_stale` percorre recursivamente `run/<id>/mysql/data` e compara o maior mtime com o do dump + 120 s. Ordem do `blocker`: rodando → pasta → dump. Um site com erro de leitura não derruba os outros.

- [ ] **Step 4: Rodar e ver passar**

Run: `cd src-tauri && cargo test --lib local_import`
Expected: todos PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/local_import.rs src-tauri/src/lib.rs
git commit -m "feat(importar-local): ler sites e estado do Local"
```

---

### Task 2: Copiar a pasta do site seguindo links

**Files:**
- Modify: `src-tauri/src/local_import.rs`

**Interfaces:**
- Produces: `pub fn copy_tree(src: &Path, dst: &Path, on_file: &dyn Fn(u64)) -> Result<Vec<String>, String>` — copia `src` para `dst` (criando `dst`), segue links simbólicos copiando o conteúdo, pula laços; `on_file` recebe a contagem acumulada de arquivos copiados (para progresso); devolve os caminhos relativos que eram links.

- [ ] **Step 1: Escrever os testes que falham**

```rust
#[test] fn copia_arquivos_e_subpastas()      // a.txt, sub/b.txt → mesmos conteúdos em dst; retorno vazio
#[test] fn link_vira_copia_e_e_listado()     // wp-content/plugins/meu -> ../../../fora/meu; dst/wp-content/plugins/meu NÃO é symlink, tem o arquivo; retorno == ["wp-content/plugins/meu"]
#[test] fn laco_de_link_nao_trava()          // sub/loop -> .. ; termina, retorno contém "sub/loop"
#[test] fn origem_fica_intacta()             // após copiar e escrever em dst, arquivos e links da origem continuam iguais (inclui o link apontando para o mesmo alvo)
```
(Testes de link com `#[cfg(unix)]`.)

- [ ] **Step 2: Rodar e ver falhar** — `cargo test --lib local_import::tests::copia` etc. → não compila.

- [ ] **Step 3: Implementar** — recursão mantendo o conjunto de pastas ancestrais canonicalizadas; um link cujo alvo canonicalizado já está nesse conjunto é laço (pula e registra). Links quebrados são pulados e registrados.

- [ ] **Step 4: Rodar e ver passar** — `cd src-tauri && cargo test --lib local_import` → PASS.

- [ ] **Step 5: Commit** — `git commit -m "feat(importar-local): cópia da pasta seguindo links"`

---

### Task 3: Restauração com origem "pasta"

**Files:**
- Modify: `src-tauri/src/backup.rs:427-490` (`restore_into`)
- Modify: `src-tauri/src/project.rs` (`pick_port` + teste)
- Modify: `src-tauri/src/commands.rs:1046-1112` (`project_from_manifest`), `1113-1220` (`restore_backup`, `restore_impl`), `1321-1343` (`create_project_from_blueprint`)
- Test: `src-tauri/src/runtime.rs` (bloco de testes `#[ignore]`, junto de `wordpress_end_to_end`, `runtime.rs:1646`)

**Interfaces:**
- Consumes: `local_import::copy_tree` (Task 2)
- Produces:
  - em `backup.rs`:
    ```rust
    pub enum RestoreSource<'a> {
        Archive(&'a Path),
        /// Pasta pública pronta + dump SQL (MySQL). Usado pelo importador do Local.
        Folder { public: &'a Path, dump: &'a Path },
    }
    pub fn restore_into(base: &Path, p: &Project, source: RestoreSource, key: &str,
                        progress: &dyn Fn(InstallProgress)) -> Result<Option<String>, String>
    ```
    `Folder`: limpa `runtime::app_dir`, copia `public` para `runtime::docroot_dir(base, p)` com `copy_tree` (passo "Copiando os arquivos…"), importa `dump` com `import_database` ("Importando o banco de dados…"), roda `wp::retarget` como hoje. Se houve links, o aviso devolvido lista-os: "Pastas que eram links no Local foram copiadas: <lista>." (concatenado com o aviso do WordPress, se houver).
  - em `commands.rs`:
    ```rust
    enum Origin { Archive(PathBuf), Folder { public: PathBuf, dump: PathBuf } }
    async fn restore_impl(app: AppHandle, st: AppState, origin: Origin, manifest: backup::Manifest,
                         target: Option<String>, new_name: Option<String>) -> Result<RestoreResult, String>
    ```
    Os chamadores atuais leem o manifesto antes e passam `Origin::Archive`. `restore_impl` com `Origin::Folder` só é aceito com `target == None`.
  - `project_from_manifest`: porta `0` no manifesto = escolher uma porta livre (nunca usar 0).

- [ ] **Step 1: Teste de unidade da porta 0** — extrair a escolha de porta de `project_from_manifest` para `pub fn pick_port(wanted: u16, start: u16, reserved: &HashSet<u16>) -> Option<u16>` em `project.rs` e testar lá: `pick_port(0, 8000, &HashSet::new())` não é `Some(0)` e é `>= 8000`; com uma porta livre `w` (de `free_port(18900, ..)`), `pick_port(w, 8000, &HashSet::new()) == Some(w)`; com `w` em `reserved`, devolve outra.

- [ ] **Step 2: Teste ponta a ponta `#[ignore]` `local_folder_restore_end_to_end`** em `runtime.rs`, com o mesmo preparo de ferramentas do `wordpress_end_to_end` (base `/tmp/localtusk-test-wp`, reaproveita o cache):
  - monta uma pasta "do Local" temporária: instala o WordPress num projeto auxiliar, faz `mysqldump` dele para `app/sql/local.sql`, troca `siteurl`/`home` no dump para `http://snap.local` e copia os arquivos para `app/public` com `wp-config.php` usando `DB_HOST 'localhost'` e `DB_NAME 'local'`;
  - cria um projeto novo (`docroot "public"`), inicia, chama `backup::restore_into(.., RestoreSource::Folder{..}, ..)`;
  - asserts: `wp option get siteurl` == `project::site_url(&p)`; GET na home responde 200 e não contém `snap.local`; a pasta "do Local" tem o mesmo conteúdo de antes (compara lista de arquivos + mtime do `local.sql`).
  - Variante de falha no mesmo teste: dump com `THIS IS NOT SQL;` → `restore_into` devolve `Err` e a pasta de origem segue intacta.

- [ ] **Step 3: Rodar e ver falhar** — `cd src-tauri && cargo test --lib local_folder_restore_end_to_end -- --ignored --nocapture` → não compila.

- [ ] **Step 4: Implementar** `RestoreSource`, `Origin`, a mudança de `restore_impl` e `pick_port`. O resto do comportamento de `restore_impl` (projeto novo, desfazer em erro, backup de segurança com `target`) fica igual.

- [ ] **Step 5: Rodar os testes**
  - `cd src-tauri && cargo test --lib` → todos PASS (inclui os de blueprint/backup).
  - `cargo test --lib local_folder_restore_end_to_end -- --ignored --nocapture` → PASS.
  - `cargo test --lib wordpress_end_to_end -- --ignored --nocapture` → PASS (sem regressão).

- [ ] **Step 6: Commit** — `git commit -m "feat(importar-local): restauração aceita uma pasta como origem"`

---

### Task 4: Comandos Tauri do importador

**Files:**
- Modify: `src-tauri/src/local_import.rs` (escolha de versões, função pura)
- Modify: `src-tauri/src/commands.rs` (novos comandos, perto dos de blueprint)
- Modify: `src-tauri/src/lib.rs` (registrar os comandos no `invoke_handler`)

**Interfaces:**
- Consumes: Tasks 1 e 3.
- Produces:
  - `pub fn fallback_version(wanted: &str, installed: &[String]) -> Option<(String, bool)>` em `local_import.rs` — `wanted` e `installed` são séries (`"8.2"`); devolve a escolhida e `true` se ela for mais antiga que `wanted`. Regra: mesma major mais próxima (empate → a mais nova), senão a mais nova instalada.
  - ```rust
    #[derive(Serialize)] #[serde(rename_all = "camelCase")]
    pub struct VersionNeed {
        pub wanted: String,            // série, ex. "8.2"
        pub installed: bool,           // série já instalada
        pub install_version: Option<String>, // patch mais novo da série no catálogo (None se fora do catálogo/offline)
        pub fallback: Option<String>,  // série para "Usar"
        pub fallback_older: bool,
    }
    #[derive(Serialize)] #[serde(rename_all = "camelCase")]
    pub struct LocalImportCheck { pub php: VersionNeed, pub mysql: VersionNeed }
    ```
  - Comandos:
    - `list_local_sites(state) -> Result<Vec<LocalSite>, String>` — sem pasta de dados ou sem `sites.json` devolve `Err("Nenhuma instalação do Local encontrada neste computador.")`.
    - `check_local_site(state, id: String) -> Result<LocalImportCheck, String>` — usa `tools::php_installs`, `mysql_manager::installs`, `php_manager::catalog`, `mysql_manager::catalog` (falha no catálogo vira `install_version: None`).
    - `import_local_site(app, state, id: String, name: String, php_version: String, mysql_version: String) -> Result<RestoreResult, String>` — revalida o site (`find_site`; `blocker` vira `Err`), monta um `backup::Manifest` sintético (`format: 1`, `id: slugify(name)`, `name`, `docroot: "public"`, `php_version`, `db_engine: "mysql"`, `db_version: mysql_version`, `db_name: ""`, portas `0`, `site_url: ""`, `has_wordpress: true`, `created_at: now`, `kind: ""`) e chama `restore_impl(.., Origin::Folder{..}, manifest, None, Some(name))`. Os avisos do site (multisite/apache/dump) entram no `warning` final antes do aviso da restauração.

- [ ] **Step 1: Testes de `fallback_version`**

```rust
assert_eq!(fallback_version("8.2", &["8.3".into(), "8.1".into()]), Some(("8.3".into(), false))); // empate → mais nova
assert_eq!(fallback_version("8.2", &["8.1".into()]), Some(("8.1".into(), true)));
assert_eq!(fallback_version("7.4", &["8.3".into(), "8.4".into()]), Some(("8.4".into(), false))); // sem mesma major → mais nova
assert_eq!(fallback_version("8.4", &["8.0".into()]), Some(("8.0".into(), true)));
assert_eq!(fallback_version("8.2", &[]), None);
```

- [ ] **Step 2: Rodar e ver falhar** — `cargo test --lib local_import::tests::fallback` → não compila.
- [ ] **Step 3: Implementar** `fallback_version` e os três comandos; registrar em `lib.rs`.
- [ ] **Step 4: Rodar** — `cd src-tauri && cargo test --lib && cargo clippy --all-targets` → PASS, sem warnings novos.
- [ ] **Step 5: Commit** — `git commit -m "feat(importar-local): comandos para listar, checar e importar"`

---

### Task 5: Tela "Importar do Local"

**Files:**
- Create: `src/LocalImport.tsx`
- Modify: `src/api.ts` (tipos `LocalSite`, `VersionNeed`, `LocalImportCheck` espelhando os do Rust; `listLocalSites`, `checkLocalSite`, `importLocalSite`)
- Modify: `src/App.tsx` (view `"local-import"`; link "Importar do Local" no cabeçalho de "Novo projeto", ao lado de "Usar um blueprint", `App.tsx:405-413`; `restoringNew` ganha `"local"` com o texto "Importando o site do Local…")
- Modify: `src/dev-mock.ts` (três sites: um pronto, um rodando, um multisite com dump desatualizado; `check_local_site` com PHP faltando; `import_local_site` devolve um projeto com aviso)

**Interfaces:**
- Consumes: comandos da Task 4; `api.installPhp(version)` e `api.installMysql(version)` existentes, com o progresso pelo evento `install-progress` (mesmo padrão de `PhpManager.tsx`).

Comportamento:
- Lista: nome, "PHP 8.2 · MySQL 8.4", "banco salvo em 12/11 12:39" (de `dumpSavedAt`), botão **Importar**; com `blocker`, botão desativado e a mensagem embaixo; `warnings` em texto discreto.
- Erro de `listLocalSites` aparece no lugar da lista (ex.: "Nenhuma instalação do Local encontrada neste computador."); lista vazia → "O Local não tem sites."
- Campo "Nome no LocalTusk", preenchido com o nome do site escolhido.
- Ao clicar em Importar: chama `checkLocalSite`. Se `php.installed && mysql.installed`, importa direto. Senão mostra, para cada versão faltante: "O site usa PHP 8.2, que não está instalado aqui." com **[Instalar o PHP 8.2]** (se `installVersion`) e **[Usar o PHP 8.3]** (se `fallback`); com `fallbackOlder` no MySQL, o alerta "O banco pode não importar numa versão mais antiga." Depois das escolhas, instala o que foi pedido (com progresso), e chama `importLocalSite(id, name, phpSerie, mysqlSerie)`.
- Fim: como `createFromBlueprint` em `App.tsx:148` — abre o projeto e mostra `warning` se houver.

- [ ] **Step 1: Implementar** os tipos, a API, a tela e o mock.
- [ ] **Step 2: Verificar tipos** — `pnpm exec tsc --noEmit` → sem erros.
- [ ] **Step 3: Verificar no navegador** — `pnpm dev`, abrir <http://localhost:1420>, Novo projeto → Importar do Local: os três sites do mock aparecem com bloqueio/avisos corretos; a escolha de versão aparece para o site com PHP faltando; importar leva ao projeto e mostra o aviso.
- [ ] **Step 4: Commit** — `git commit -m "feat(importar-local): tela Importar do Local"`

---

### Task 6: Teste real e documentação

**Files:**
- Modify: `README.md` (item na lista "Estado atual": importar sites do Local; e o novo teste na seção de testes)

- [ ] **Step 1: Teste manual com o Local real** — `pnpm tauri dev`; com o site `snap` parado no Local, importar; conferir que o site abre em `snap.localhost:<porta>`, que o login do wp-admin funciona e que `~/Local Sites/snap` não mudou (`git status`-like: comparar `ls -la` e mtime do `local.sql` antes/depois). Com o site iniciado no Local, conferir que o botão fica desativado.
- [ ] **Step 2: Atualizar o README.**
- [ ] **Step 3: Rodar tudo** — `cd src-tauri && cargo test --lib` e `pnpm exec tsc --noEmit` → PASS.
- [ ] **Step 4: Commit** — `git commit -m "docs: importar sites do Local"`
