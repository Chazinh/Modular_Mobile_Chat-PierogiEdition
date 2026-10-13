# Changelog

As alterações do projeto serão registradas aqui. A seção **Não lançado** reúne
mudanças ainda sem uma versão publicada; análises não representam funcionalidades
implementadas.

## Não lançado

### Apresentação do README — 2026-10-13

- Incorporada a estrutura visual fornecida: saudação e duas imagens originais
  centralizadas, seguidas pelo título e pela documentação atualizada. Removida
  a apresentação duplicada e os espaços Unicode usados como alinhamento.
- Mantidos os recursos e as instruções atuais de idiomas, controles, host,
  provedores, GLM e avaliação Rust. Corrigido o trecho Android que ainda orientava
  conexão HTTP direta, alinhando-o ao gateway HTTPS e à confiança no certificado.
- Reconciliado o README após conflito de integração: removidos marcadores e
  trechos de diff incorporados ao texto; recuperados os exemplos completos de
  provedores/presets, a estrutura do projeto e as notas de segurança, mantendo
  as imagens originais, os dois idiomas e a orientação HTTPS.

### Idiomas e controles visuais — 2026-10-13

- Adicionada a tradução da interface para português brasileiro e
  inglês, com atualização imediata, preferência local `mc.language`, sincronização
  entre abas e textos de biometria Android. Conteúdo das conversas, fichas,
  presets e regras de idioma das respostas não são alterados pela preferência.
- Substituídos os seletores nativos de idioma/tipo de memória e a janela de cor
  por folhas com o vidro, a paleta e os botões do aplicativo. A cor personalizada
  mantém entrada HEX e controles de matiz, saturação e luminosidade.
- Confirmações de exclusão/limpeza passam a usar folhas do app, com cancelamento
  por Voltar/Escape e execução somente após confirmação. Erros de edição da
  ficha aparecem na própria página. Adicionados foco de teclado e semântica de
  diálogo às folhas; a camada superior recebe Escape/Tab.
- Corrigida a sobreposição das folhas abertas dentro de cartões: elas passam a
  ser renderizadas acima da tela inteira, evitando que os controles seguintes
  cubram o seletor. Os novos painéis usam vidro mais opaco para leitura clara.
- Tema nativo Android ajustado para as cores escuras e lavanda do app, inclusive
  avisos de biometria, preservando a autenticação forte do sistema.
- README atualizado para os dois idiomas e saudação/imagens dos ícones
  centralizadas com HTML compatível com GitHub.

### Host privado e menu — 2026-10-13

- Incorporado o suporte genérico a hospedagem HTTPS com template Caddy, login
  próprio do navegador, verificação do token Bearer no backend e bloqueio da
  criação de códigos de pareamento pelo proxy.
- Corrigidos os redirecionamentos relativos de login no template Caddy com
  matcher explícito e preservadas as pré-verificações CORS do cliente Android.
- Adicionados helpers Windows para preparar/controlar o Caddy, atualizar DuckDNS
  e gerenciar somente encaminhamentos IPv4 TCP 80/443 pertencentes à configuração.
  Domínio, IP, sub-rede, diretório e credenciais pessoais não foram importados.
- Adicionada a opção `-Loopback` para executar Rust atrás do proxy sem expô-lo à LAN.
- Conexão Android passa a aceitar endereços HTTPS local e remoto do mesmo PC,
  com sondagens limitadas, escolha compartilhada por até 30 segundos e reavaliação
  no retorno, em mudanças de rede e em falhas. POSTs e fluxos abertos conservam
  seu destino e não são repetidos pela seleção de endpoint.
- Android bloqueia HTTP externo e conteúdo misto. Confiança em CA privada é
  opcional e restrita ao host configurado, gerada fora dos arquivos versionados;
  nenhuma CA do host de origem ou chave privada acompanha o pacote padrão.
  A política do aplicativo prevalece sobre a configuração HTTP de plugins no
  manifesto combinado.
- Portada a correção final do menu lateral: mola mais suave, limiar de abertura
  de 12 pixels, controles excluídos do gesto de borda, limpeza de eventos antigos
  e proteção contra animação após destruição. Preservados os fluxos de abertura
  por botão, Voltar, arrasto, folhas inferiores e estilos existentes.
- Atualizados README e OPERACAO.md com instruções portáteis, requisitos de
  certificado, pareamento local e limites de CGNAT/IPv6. Entregas e configurações
  particulares da instalação de origem não representam instalação deste projeto.
- Adicionadas verificações isoladas de seleção, cancelamento, preservação de POST,
  menu, parâmetros de host, recursos Android e gateway Caddy opcional.
- Verificados build web, traduções/interpolação, conversão de cores, fluxos do
  menu em navegador sem janela, gateway Caddy isolado e compilação de Java/
  recursos Android nos modos padrão e CA opcional. Instalação em aparelho,
  emissão de certificados públicos, DNS e encaminhamento real de portas não
  foram executados nesta atualização.

### Adicionado — 2026-10-13

- Changelog do projeto e link de acesso no README.
- Política de migração para Rust no README, com prioridades, riscos e critérios
  para aceitar futuras mudanças.

### Avaliado — 2026-10-13

- Confirmado no código que API HTTP, geração/SSE, contexto, memória, jobs,
  persistência SQLite e cofre criptografado já estão implementados em Rust.
- Identificadas como candidatas a avaliação futura: instalação/verificação de
  modelos, partes do inicializador Windows e testes/benchmarks intensivos que
  hoje usam comunicação entre Node e Rust.
- Parser de cenas considerado candidato condicional a Rust/WebAssembly, somente
  se medições no cliente demonstrarem um gargalo e ganho no percurso completo.
- Recomendada a manutenção de React, gestos/DOM, transporte do cliente, cache do
  navegador e integrações Android com Keystore/biometria nas tecnologias atuais.
- Nenhuma migração de código de execução foi aplicada nesta revisão. Ganhos de
  desempenho, RAM ou VRAM de uma futura migração ainda não foram demonstrados.

### Alterado — 2026-10-13

- Prioridades da avaliação documentadas: recuperar a massa de teste ausente,
  medir os gargalos atuais, avaliar o instalador de modelos, reduzir IPC em
  testes intensivos e só depois considerar inicializador e parser Wasm.

### Problemas conhecidos — 2026-10-13

- `tests/fixtures/world-card.txt` está vazio nesta cópia do projeto. O teste Rust
  `structured_card_matches_world_contract` falha por não encontrar os personagens
  esperados. É necessário recuperar a ficha fictícia original antes de afirmar
  que a base de testes está completamente verde.
