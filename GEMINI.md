# Diretrizes Locais do Projeto — Modular-Mobile-Chat

## Prevenção de Regressão e Validação
- **Todo bug corrigido vira um teste automatizado novo para evitar regressão.**
- Para cada correção efetuada (em Rust, scripts, cliente ou comunicação), crie um teste automatizado correspondente cobrindo o defeito para garantir que a regressão seja impedida.
