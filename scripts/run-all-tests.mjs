#!/usr/bin/env node
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const ROOT = path.resolve(__dirname, '..');

// Formatação ANSI para terminal
const colors = {
  reset: '\x1b[0m',
  bold: '\x1b[1m',
  dim: '\x1b[2m',
  red: '\x1b[31m',
  green: '\x1b[32m',
  yellow: '\x1b[33m',
  blue: '\x1b[34m',
  cyan: '\x1b[36m',
};

function log(msg = '') {
  console.log(msg);
}

function header(title) {
  log(`\n${colors.bold}${colors.cyan}=== ${title} ===${colors.reset}`);
}

function parseArgs(args) {
  const flags = {
    all: false,
    unit: false,
    integration: false,
    rust: false,
    build: false,
    bail: false,
    filter: null,
    help: false,
  };

  for (const arg of args) {
    if (arg === '--all') flags.all = true;
    else if (arg === '--unit' || arg === '--fast') flags.unit = true;
    else if (arg === '--integration') flags.integration = true;
    else if (arg === '--rust') flags.rust = true;
    else if (arg === '--build') flags.build = true;
    else if (arg === '--bail') flags.bail = true;
    else if (arg.startsWith('--filter=')) flags.filter = arg.slice(9);
    else if (arg === '-h' || arg === '--help') flags.help = true;
  }

  // Se nenhum subconjunto específico foi pedido, define comportamento padrão
  if (!flags.all && !flags.unit && !flags.integration && !flags.rust && !flags.build && !flags.filter) {
    // Por padrão roda tudo
    flags.all = true;
  }

  return flags;
}

function showHelp() {
  log(`${colors.bold}Automação de Testes - ModularChat (Pierogi Edition)${colors.reset}`);
  log(`Uso: node scripts/run-all-tests.mjs [opções]\n`);
  log(`Opções:`);
  log(`  --all          Executa todas as suítes (unitários, integração, Rust e build)`);
  log(`  --unit, --fast Executa apenas os testes unitários rápidos (*.test.js)`);
  log(`  --integration  Executa testes de integração (rust-runtime e rust-privacy)`);
  log(`  --rust         Executa os testes do backend em Rust (cargo test)`);
  log(`  --build        Executa a validação de compilação do cliente (vite build)`);
  log(`  --filter=<txt> Filtra arquivos de teste cujo nome contenha o texto indicado`);
  log(`  --bail         Interrompe a execução na primeira falha`);
  log(`  -h, --help     Exibe esta ajuda\n`);
  log(`Exemplos:`);
  log(`  node scripts/run-all-tests.mjs --unit`);
  log(`  node scripts/run-all-tests.mjs --filter=server-address`);
  log(`  node scripts/run-all-tests.mjs --all`);
}

function runProcess(cmd, args, options = {}) {
  return new Promise((resolve) => {
    const t0 = Date.now();
    let executable = cmd;
    if (process.platform === 'win32' && cmd === 'npm') {
      executable = 'npm.cmd';
    }
    const proc = spawn(executable, args, {
      cwd: options.cwd || ROOT,
      env: { ...process.env, ...options.env },
      stdio: 'inherit',
      shell: process.platform === 'win32' && (cmd === 'npm' || executable.endsWith('.cmd')),
    });

    proc.on('close', (code) => {
      resolve({
        code: code ?? 1,
        duration: Date.now() - t0,
      });
    });

    proc.on('error', (err) => {
      console.error(`${colors.red}Erro ao iniciar processo ${cmd}:${colors.reset}`, err.message);
      resolve({
        code: 1,
        duration: Date.now() - t0,
        error: err,
      });
    });
  });
}

async function main() {
  const flags = parseArgs(process.argv.slice(2));

  if (flags.help) {
    showHelp();
    process.exit(0);
  }

  log(`${colors.bold}${colors.blue}>>> Inicializando Automação Geral de Testes...${colors.reset}`);
  log(`${colors.dim}Data/Hora: ${new Date().toISOString()}${colors.reset}`);
  log(`${colors.dim}Diretório Raiz: ${ROOT}${colors.reset}`);

  const results = [];

  // 1. Descoberta e Execução de Testes Unitários em JS
  if (flags.all || flags.unit || flags.filter) {
    header('Testes Unitários JavaScript (tests/*.test.js)');
    const testsDir = path.join(ROOT, 'tests');
    let testFiles = fs.readdirSync(testsDir)
      .filter((file) => file.endsWith('.test.js'))
      .map((file) => path.join('tests', file));

    if (flags.filter) {
      testFiles = testFiles.filter((f) => f.toLowerCase().includes(flags.filter.toLowerCase()));
      log(`${colors.yellow}Filtro aplicado ("${flags.filter}"): ${testFiles.length} arquivos selecionados.${colors.reset}`);
    }

    if (testFiles.length === 0) {
      log(`${colors.yellow}Nenhum arquivo de teste encontrado para o critério selecionado.${colors.reset}`);
    } else {
      log(`${colors.dim}Executando ${testFiles.length} suítes com node --test...${colors.reset}\n`);

      const nodeArgs = ['--test', '--test-concurrency=1', ...testFiles];
      const res = await runProcess('node', nodeArgs);

      results.push({
        name: `Testes Unitários JS (${testFiles.length} arquivos)`,
        code: res.code,
        duration: res.duration,
      });

      if (res.code !== 0 && flags.bail) {
        finishAndExit(results);
        return;
      }
    }
  }

  // 2. Testes de Integração JS / Contratos Rust HTTP
  if ((flags.all || flags.integration) && !flags.filter) {
    header('Testes de Integração de Runtime & Privacidade Rust');
    const integrationFiles = [
      'tests/rust-runtime.mjs',
      'tests/rust-privacy.mjs',
    ];

    log(`${colors.dim}Executando testes de integração: ${integrationFiles.join(', ')}...${colors.reset}\n`);

    const nodeArgs = ['--test', '--test-concurrency=1', ...integrationFiles];
    const res = await runProcess('node', nodeArgs);

    results.push({
      name: 'Integração Runtime & Privacidade (Node + Rust)',
      code: res.code,
      duration: res.duration,
    });

    if (res.code !== 0 && flags.bail) {
      finishAndExit(results);
      return;
    }
  }

  // 3. Testes Nativos do Backend Rust (Cargo test)
  if ((flags.all || flags.rust) && !flags.filter) {
    header('Testes Nativos do Servidor Rust (cargo test)');
    log(`${colors.dim}Executando cargo test --manifest-path server-rust/Cargo.toml...${colors.reset}\n`);

    const res = await runProcess('cargo', ['test', '--manifest-path', 'server-rust/Cargo.toml']);

    results.push({
      name: 'Testes do Servidor Rust (cargo test)',
      code: res.code,
      duration: res.duration,
    });

    if (res.code !== 0 && flags.bail) {
      finishAndExit(results);
      return;
    }
  }

  // 4. Verificação de Compilação do Cliente Vite
  if ((flags.all || flags.build) && !flags.filter) {
    header('Verificação de Compilação do Cliente (vite build)');
    log(`${colors.dim}Executando npm run build no diretório client/...${colors.reset}\n`);

    const res = await runProcess('npm', ['run', 'build'], {
      cwd: path.join(ROOT, 'client'),
    });

    results.push({
      name: 'Compilação de Produção do Cliente (client)',
      code: res.code,
      duration: res.duration,
    });

    if (res.code !== 0 && flags.bail) {
      finishAndExit(results);
      return;
    }
  }

  finishAndExit(results);
}

function finishAndExit(results) {
  log(`\n${colors.bold}${colors.cyan}========================================${colors.reset}`);
  log(`${colors.bold}        RELATÓRIO FINAL DE TESTES        ${colors.reset}`);
  log(`${colors.bold}${colors.cyan}========================================${colors.reset}`);

  let allPassed = true;
  let totalTime = 0;

  for (const r of results) {
    totalTime += r.duration;
    const durSec = (r.duration / 1000).toFixed(2);
    if (r.code === 0) {
      log(` ${colors.green}✔ [PASS]${colors.reset} ${r.name} ${colors.dim}(${durSec}s)${colors.reset}`);
    } else {
      allPassed = false;
      log(` ${colors.red}✖ [FAIL]${colors.reset} ${r.name} ${colors.dim}(${durSec}s)${colors.reset}`);
    }
  }

  log(`${colors.cyan}----------------------------------------${colors.reset}`);
  const totalSec = (totalTime / 1000).toFixed(2);
  log(`Tempo Total: ${totalSec}s | Suítes: ${results.length}`);

  if (allPassed) {
    log(`${colors.bold}${colors.green}Resultado Geral: SUCESSO - Todos os testes foram aprovados!${colors.reset}\n`);
    process.exit(0);
  } else {
    log(`${colors.bold}${colors.red}Resultado Geral: FALHA - Uma ou mais suítes apresentaram erros.${colors.reset}\n`);
    process.exit(1);
  }
}

main().catch((err) => {
  console.error(`${colors.red}Erro fatal na automação de testes:${colors.reset}`, err);
  process.exit(1);
});
