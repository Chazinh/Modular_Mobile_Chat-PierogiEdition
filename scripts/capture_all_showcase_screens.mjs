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

  // Consistent mobile viewport
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
  await new Promise(r => setTimeout(r, 1000));

  // Set tokens and language to EN
  await send('Runtime.evaluate', {
    expression: `
      localStorage.setItem('mc.token', '${TOKEN}');
      localStorage.setItem('mc.server', 'http://127.0.0.1:3000');
      localStorage.setItem('mc.language', 'en');
    `
  });
  await send('Page.reload');
  await new Promise(r => setTimeout(r, 1500));

  async function snap(filename) {
    const res = await send('Page.captureScreenshot', { format: 'png' });
    fs.writeFileSync(filename, Buffer.from(res.data, 'base64'));
    console.log(`Saved ${filename}`);
  }

  // 1. Home
  console.log('Capturing screen_01_home.png...');
  await snap('screen_01_home.png');

  // 2. Open chat
  console.log('Opening chat...');
  await send('Runtime.evaluate', {
    expression: `
      const item = document.querySelector('.mc-chat-item') || Array.from(document.querySelectorAll('button')).find(b => b.innerText.includes('TEST'));
      if (item) item.click();
    `
  });
  await new Promise(r => setTimeout(r, 1200));

  // Scroll smoothly down a bit so the top bubble doesn't overlap text
  await send('Runtime.evaluate', {
    expression: `
      const scroller = document.querySelector('.mc-scroller');
      if (scroller) scroller.scrollTop = 140;
    `
  });
  await new Promise(r => setTimeout(r, 500));

  console.log('Capturing screen_02_chat.png...');
  await snap('screen_02_chat.png');

  // 3. Open Drawer
  console.log('Opening drawer...');
  await send('Runtime.evaluate', {
    expression: `
      const opt = document.querySelector('button[aria-label="Options"], button[aria-label="Opções"]');
      if (opt) opt.click();
    `
  });
  await new Promise(r => setTimeout(r, 1000));

  console.log('Capturing screen_03_drawer.png...');
  await snap('screen_03_drawer.png');

  // 4. Open Models
  console.log('Opening models page...');
  await send('Runtime.evaluate', {
    expression: `
      const mBtn = Array.from(document.querySelectorAll('.mc-drawer button')).find(b => b.innerText.includes('Model & effort') || b.innerText.includes('Modelo'));
      if (mBtn) mBtn.click();
    `
  });
  await new Promise(r => setTimeout(r, 1000));

  console.log('Capturing screen_04_models.png...');
  await snap('screen_04_models.png');

  ws.close();
  console.log('Done capturing all 4 showcase screens in English!');
}

main().catch(console.error);
