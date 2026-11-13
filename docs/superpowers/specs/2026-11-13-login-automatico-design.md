# Login automático no wp-admin — design

Data: 2026-11-13 · Fork do LocalTusk

## Intenção

Como o "One-click admin" do Local: o botão **wp-admin** do projeto já entra logado no WordPress,
sem digitar usuário e senha. Útil principalmente para sites importados do Local, cuja senha de
admin muitas vezes ninguém lembra.

## Objetivos

- Por projeto, uma chave **liga/desliga** e um **seletor do usuário administrador** (decidido com
  o usuário, opção B).
- Ligado: clicar em wp-admin abre o navegador já logado como o usuário escolhido.
- Desligado: o botão volta a abrir `/wp-admin/` como hoje (tela de login).

## Fora do escopo

- Login automático no front (só o botão wp-admin).
- Multisite: funciona como login comum no site principal; nada específico de rede.

## Anti-objetivos

- Dar acesso de admin a quem não clicou no app (o projeto pode estar aberto na rede: `lan`).
- Quebrar o logout ou o teste do site como visitante.
- Escrever qualquer coisa fora do projeto do LocalTusk (nunca nas pastas do Local).

## Abordagens

1. **Código de uso único (escolhida).** No clique, o app gera um código aleatório, guarda no banco
   do site por wp-cli (válido por 60 s) e abre `/?localtusk-login=<código>`. Um mu-plugin confere,
   apaga o código, faz o login e redireciona para `/wp-admin/`. Seguro mesmo com acesso pela rede;
   logout normal. Custo: ~1 s a mais no clique.
2. Login sempre automático por mu-plugin: instantâneo, mas qualquer aparelho da rede vira admin e
   o logout para de funcionar. Descartada.
3. App gerar o cookie de login: o app não controla os cookies do navegador. Descartada.

## Como funciona

- **Configuração no projeto** (`Project`, com `#[serde(default)]` para projetos antigos):
  `auto_login: bool` (padrão desligado) e `auto_login_user: String` (o `user_login`).
- **Tela** (painel WordPress do projeto, `src/WordPress.tsx`), quando o WordPress está instalado:

  ```
  [x] Login automático no wp-admin   Usuário: [admin ▾]
  ```

  - A lista vem dos usuários com papel administrador (`wp user list --role=administrator`) e só
    carrega com o projeto rodando; parado, mostra o usuário salvo e o aviso que já existe
    ("Inicie o projeto…").
  - Ao ligar sem usuário salvo, escolhe o primeiro administrador da lista. *(Chamada minha.)*
- **mu-plugin** `wp-content/mu-plugins/localtusk-login.php` (pasta obtida do próprio WordPress):
  instalado ao ligar e conferido a cada clique (se alguém apagou, volta); removido ao desligar.
  Lê `?localtusk-login=`, procura o código pelo hash (SHA-256) num transient, apaga na hora
  (uso único), confere que o usuário ainda é administrador, faz `wp_set_auth_cookie` e redireciona
  para `/wp-admin/`. Código inválido ou vencido: redireciona para `wp-login.php` sem erro técnico.
- **Clique em wp-admin com login ligado:** comando Tauri novo que gera um código de 32 bytes
  aleatórios, grava o transient (hash → usuário, 60 s), garante o mu-plugin e abre a URL.
  - Usuário salvo não existe mais ou deixou de ser admin: abre a tela de login normal e mostra
    "O usuário <nome> não é mais administrador deste site. Escolha outro no login automático."
  - Falha do wp-cli: abre a tela de login normal e mostra o erro.

## Testes

- Unidade: valores padrão da configuração e leitura de projetos antigos sem os campos.
- Ponta a ponta `#[ignore]` (estilo dos existentes): WordPress instalado, liga o login, gera um
  código, `GET /?localtusk-login=<código>` devolve redirect com cookie de login e o wp-admin abre
  com esse cookie; o mesmo código usado de novo não loga; desligar remove o mu-plugin.

## Deixado para quem for construir

- Nomes exatos dos comandos e funções, e como rodar o wp-cli (reaproveitar o que `wp.rs` já faz).
- Layout exato da chave e do seletor, seguindo o padrão do painel.
