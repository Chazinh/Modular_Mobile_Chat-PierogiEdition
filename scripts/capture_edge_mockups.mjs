import fs from 'fs';

const TOKEN = 'de77285e7d3e4fe58faf9537960e8ae84f31cce57da042c0832668178f060e08';

async function main() {
  const pages = await fetch('http://127.0.0.1:9333/json').then(r => r.json());
  const page = pages.find(p => p.type === 'page');
  if (!page) throw new Error('No page found');

  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise(r => ws.onopen = r);

  let id = 1;
  function send(method, params = {}) {
    return new Promise((resolve, reject) => {
      const curId = id++;
      const handler = (event) => {
        const msg = JSON.parse(event.data);
        if (msg.id === curId) {
          ws.removeEventListener('message', handler);
          if (msg.error) reject(new Error(msg.error.message));
          else resolve(msg.result);
        }
      };
      ws.addEventListener('message', handler);
      ws.send(JSON.stringify({ id: curId, method, params }));
    });
  }

  // Set device emulation
  await send('Emulation.setDeviceMetricsOverride', {
    width: 412,
    height: 915,
    deviceScaleFactor: 2.625,
    mobile: true,
  });
  await send('Emulation.setTouchEmulationEnabled', { enabled: true });
  await send('Page.enable');
  await send('Runtime.enable');

  // Go to 127.0.0.1:3000
  await send('Page.navigate', { url: 'http://127.0.0.1:3000/' });
  await new Promise(r => setTimeout(r, 1500));

  // Set token & language
  await send('Runtime.evaluate', {
    expression: `
      localStorage.setItem('mc.token', '${TOKEN}');
      localStorage.setItem('mc.server', 'http://127.0.0.1:3000');
      localStorage.setItem('mc.language', 'pt-BR');
    `
  });

  // Reload
  await send('Page.reload');
  await new Promise(r => setTimeout(r, 2000));

  // Function to capture screenshot
  async function snap(filename) {
    const res = await send('Page.captureScreenshot', { format: 'png' });
    fs.writeFileSync(filename, Buffer.from(res.data, 'base64'));
    console.log(`Saved ${filename} (${res.data.length} b64 chars)`);
  }

  // 1. Home screen
  console.log('Capturing 01_home.png...');
  await snap('mockup_01_home.png');

  // 2. Open chat
  console.log('Opening chat...');
  await send('Runtime.evaluate', {
    expression: `document.querySelectorAll('.mc-chat-item, [role="button"], button')[3]?.click()`
  });
  await new Promise(r => setTimeout(r, 1200));

  // Show topbar in chat
  await send('Runtime.evaluate', {
    expression: `document.querySelector('button[aria-label*="barra"], button[aria-label*="bar"]')?.click()`
  });
  await new Promise(r => setTimeout(r, 600));

  console.log('Capturing 02_chat.png...');
  await snap('mockup_02_chat.png');

  // 3. Open Drawer
  console.log('Opening drawer...');
  await send('Runtime.evaluate', {
    expression: `document.querySelector('button[aria-label="Opções"], button[aria-label="Options"]')?.click()`
  });
  await new Promise(r => setTimeout(r, 1000));

  console.log('Capturing 03_drawer.png...');
  await snap('mockup_03_drawer.png');

  // 4. Open Models page
  console.log('Opening models page...');
  await send('Runtime.evaluate', {
    expression: `Array.from(document.querySelectorAll('.mc-drawer button')).find(b => b.innerText.includes('Modelo') || b.innerText.includes('Model'))?.click()`
  });
  await new Promise(r => setTimeout(r, 1000));

  console.log('Capturing 04_models.png...');
  await snap('mockup_04_models.png');

  // 5. Back and open Home Drawer / Settings
  await send('Runtime.evaluate', {
    expression: `document.querySelector('button[aria-label*="Voltar"], button[aria-label*="Back"]')?.click()`
  });
  await new Promise(r => setTimeout(r, 600));
  // Back to home
  await send('Runtime.evaluate', {
    expression: `document.querySelector('button[aria-label*="Voltar"], button[aria-label*="Back"]')?.click()`
  });
  await new Promise(r => setTimeout(r, 1000));

  // Open Home Drawer (settings gear)
  await send('Runtime.evaluate', {
    expression: `document.querySelector('button[aria-label="Configurações"], button[aria-label="Settings"]')?.click()`
  });
  await new Promise(r => setTimeout(r, 1000));

  console.log('Capturing 05_homedrawer.png...');
  await snap('mockup_05_homedrawer.png');

  ws.close();
  console.log('All mockups captured successfully!');
}

main().catch(console.error);
