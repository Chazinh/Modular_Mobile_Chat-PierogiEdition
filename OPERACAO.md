# Hospedagem privada e conexão HTTPS

Este guia descreve os recursos portáteis do ModularChat. Nenhum domínio, IP,
credencial, certificado ou identificador de aparelho pessoal acompanha o
projeto. Os exemplos usam valores fictícios; escolha os endereços do seu host.

## Funcionamento

O PC executa o servidor Rust e guarda o cofre. O Caddy pode oferecer HTTPS na
rede local e em um domínio público. O APK recebe dois endereços configuráveis do
mesmo PC: um local e outro para acesso pela internet. Nenhum endereço remoto é
preenchido automaticamente.

Antes de uma requisição, o app consulta apenas a saúde dos endereços: prefere o
local, com limite de 1,5 segundo, e tenta o remoto, com limite de 5 segundos.
A escolha é conservada por até 30 segundos e reavaliada ao retornar ao app,
quando a rede muda ou depois de uma falha. Uma requisição ou fluxo já aberto
conserva seu endereço. POSTs de gravação/geração não são repetidos pela seleção;
a recuperação de geração continua usando `opId`, geração e offsets do servidor.

O Caddy separa o login do navegador do Bearer de pareamento do app. O backend
verifica o token antes do encaminhamento de uma API autenticada. Solicitações
CORS de pré-verificação são conferidas pelo backend. A criação de códigos de
pareamento fica bloqueada no proxy; gere o código diretamente no PC.

## Preparar o host Windows

1. Instale um binário oficial do Caddy e confira sua integridade conforme a
   publicação oficial. Coloque `caddy.exe` no diretório escolhido para o host.
   Por padrão, os scripts usam `$env:USERPROFILE\ModularChat-local\caddy`.
2. Compile o projeto seguindo o README. Inicie o Rust com
   `start-server.ps1 -Loopback` para que ele aceite somente conexões do próprio
   PC quando estiver atrás do proxy. O Caddy não inicia Rust nem o provedor.
3. Prepare a configuração e, se desejar, instale o serviço **ModularChatCaddy**.
   A instalação do serviço e mudanças de firewall exigem PowerShell elevado.

```powershell
$hosting = Join-Path $env:USERPROFILE 'ModularChat-local\caddy'
.\scripts\caddy-host.ps1 -Action Prepare -Base $hosting
.\scripts\caddy-host.ps1 -Action Install -Base $hosting
```

`Prepare` gera uma senha aleatória, um hash Argon2id e um token de sessão local,
valida a configuração e não substitui um serviço existente. A senha entra no
Caddy pelo stdin. `login.txt` contém as credenciais do navegador, protegido por
ACL; ele não pertence ao repositório. Os arquivos `auth.json`, `Caddyfile` e
`data` também precisam permanecer privados.

Os parâmetros `-BackendPort`, `-HttpsPort`, `-AdminPort` e `-WebUser` permitem
adaptar portas e usuário do login sem editar o código. Os padrões são 3000,
8443, 2019 e `modularchat`. As três portas devem ser distintas. O diretório
`-Base` precisa ser o mesmo em todas as operações desse host.

## HTTPS somente na rede local

```powershell
.\scripts\caddy-host.ps1 -Action Lan -Base $hosting `
  -LanAddress '192.168.0.10' -LanSubnet '192.168.0.0/24'
```

Substitua esses valores pela sua rede. O script aceita um IPv4 privado e uma
sub-rede que contenha esse IP. A regra de firewall permite apenas a sub-rede
informada e o perfil privado. O certificado LAN é emitido pela CA interna desse
host; para o APK confiar nele, use os recursos opcionais descritos abaixo.

`Local` retorna a configuração a acesso apenas no próprio PC e desativa as
regras de entrada e a tarefa DuckDNS desse projeto. `Trust` adiciona a CA interna
ao usuário atual do Windows, quando explicitamente executado; não afeta Android.
`Status`, a ação padrão, consulta o estado. `Stop` para apenas o serviço Caddy.

## Domínio público e DNS dinâmico

Configure um domínio seu ou um subdomínio gratuito. Depois de configurar os
registros DNS e a conectividade de entrada do roteador:

```powershell
.\scripts\caddy-host.ps1 -Action Activate -Base $hosting `
  -Domain 'chat.example.com' `
  -LanAddress '192.168.0.10' -LanSubnet '192.168.0.0/24'
```

O domínio de exemplo não é um host utilizável. O Caddy obtém e renova um
certificado público quando as condições de DNS e acesso às portas de validação
forem atendidas. O script não garante que o provedor de internet permita acesso
de entrada. Não publique diretamente o Rust ou o roteador de modelos.

Para DuckDNS, coloque somente o token em `duckdns-token.txt` no diretório privado
preparado e informe o seu subdomínio. O arquivo não é enviado para o APK.

```powershell
.\scripts\duckdns-host.ps1 -Action Update -Subdomain 'seu-subdominio' -Base $hosting
.\scripts\duckdns-host.ps1 -Action Enable -Subdomain 'seu-subdominio' -Base $hosting
```

`Enable` instala uma tarefa oculta de atualização a cada cinco minutos, sem o
token nos argumentos. Ela não substitui uma tarefa existente. `Disable` mantém
os registros e desativa a tarefa. O atualizador registra diagnósticos locais
com o token removido de mensagens de erro.

O helper `caddy-portas.ps1` opera somente encaminhamentos IPv4 TCP 80/443 por
UPnP. Sua ação padrão é `Status`; `Apply` é explícito e recusa destinos
conflitantes. `Remove` só considera as regras registradas no manifesto próprio.
Ele não configura regras IPv6 nem contorna CGNAT. Em CGNAT, pode ser necessário
IPv4 público, conectividade IPv6 adequada ou outra solução de acesso.

## Pareamento e confiança Android

Informe o endereço HTTPS local e, opcionalmente, o endereço HTTPS pela internet
na tela de conexão. Os dois devem chegar ao mesmo PC. Faça o primeiro pareamento
na LAN. Gere o código no PC:

```powershell
.\scripts\caddy-host.ps1 -Action PairCode -Base $hosting
```

Endereços HTTP antigos são reconhecidos como configurações que precisam de
HTTPS. O aplicativo não adivinha a porta ou o certificado de outro host. Uma
tentativa de pareamento malsucedida restaura a configuração de endereço anterior.
Trocar de PC remove a associação com o domínio do PC anterior.

O Android usa as CAs do sistema por padrão. HTTP externo é bloqueado;
`localhost`, a origem interna do Capacitor, é a única exceção. A verificação TLS
nunca é desligada. Se o endereço LAN já possui um certificado confiável pelo
sistema, nenhum recurso adicional é necessário.

Para uma CA interna, gere recursos de confiança restritos ao host, usando
somente o certificado **público** da CA. O helper rejeita chaves privadas e
arquivos PKCS#12. O diretório padrão é ignorado pelo Git.

```powershell
.\scripts\configure-android-host.ps1 -HostName '192.168.0.10' `
  -CertificatePath (Join-Path $hosting 'data\pki\authorities\local\root.crt')
$env:MC_ANDROID_HOST_TRUST_DIR = (Resolve-Path 'client\android\host-trust').Path
```

Depois, compile e sincronize o cliente conforme o README e faça o build Android
com essa variável presente. O Gradle inclui os recursos locais apenas nesse
build. Sem a variável, o pacote não contém certificado ou endereço particular.
Não publique a pasta de recursos específicos do host nem arquivos de assinatura.

## Preservação e limites

- Preserve o cofre e sua chave DPAPI, além de `data` do Caddy, que contém chaves
  e certificados do host. Os scripts de proxy não substituem dados do app.
- O PC precisa estar ligado, com Rust e o provedor disponíveis. Renovação TLS
  não inicia esses processos nem evita suspensão do PC.
- O cofre e o cache permanecem criptografados; HTTPS protege o transporte.
  O PC e o modelo escolhido recebem o texto necessário para responder.
- O domínio não corrige regras IPv6 do roteador quando o prefixo muda. UPnP pode
  perder regras após reinício; verifique a conectividade sem assumir sucesso.
- Compilação e testes isolados não comprovam funcionamento em uma instalação
  específica ou em troca física entre Wi-Fi e rede móvel.

Referências: [Caddy forward_auth](https://caddyserver.com/docs/caddyfile/directives/forward_auth),
[HTTPS automático do Caddy](https://caddyserver.com/docs/automatic-https),
[segurança de rede Android](https://developer.android.com/privacy-and-security/security-config)
e [especificação DuckDNS](https://www.duckdns.org/spec.jsp).
