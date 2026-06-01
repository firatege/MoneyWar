const pty = require('node-pty');
const WebSocket = require('ws');
const http = require('http');

const HTML = `<!DOCTYPE html>
<html>
<head>
  <meta charset="utf-8">
  <title>MoneyWar</title>
  <link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/xterm/css/xterm.css"/>
  <style>
    * { margin: 0; padding: 0; box-sizing: border-box; }
    body { background: #1a1a1a; display: flex; flex-direction: column; align-items: center; justify-content: center; min-height: 100vh; }
    #terminal { padding: 8px; }
    #status { color: #666; font-family: monospace; font-size: 12px; padding: 4px 8px; }
  </style>
</head>
<body>
  <div id="status">Bağlanıyor...</div>
  <div id="terminal"></div>
  <script src="https://cdn.jsdelivr.net/npm/xterm/lib/xterm.js"></script>
  <script src="https://cdn.jsdelivr.net/npm/xterm-addon-fit/lib/xterm-addon-fit.js"></script>
  <script>
    const term = new Terminal({ cursorBlink: true, fontSize: 14, theme: { background: '#1a1a1a' } });
    const fitAddon = new FitAddon.FitAddon();
    term.loadAddon(fitAddon);
    term.open(document.getElementById('terminal'));
    fitAddon.fit();

    const ws = new WebSocket((location.protocol === 'https:' ? 'wss://' : 'ws://') + location.host + '/ws');
    ws.binaryType = 'arraybuffer';

    ws.onopen = () => {
      document.getElementById('status').textContent = '● Bağlandı — yeni oturum';
      // Boyut bilgisini gönder
      ws.send(JSON.stringify({ type: 'resize', cols: term.cols, rows: term.rows }));
    };

    ws.onmessage = (e) => {
      if (e.data instanceof ArrayBuffer) {
        term.write(new Uint8Array(e.data));
      } else {
        term.write(e.data);
      }
    };

    ws.onclose = () => {
      document.getElementById('status').textContent = '○ Bağlantı kesildi — sayfayı yenile';
      term.write('\\r\\n\\x1b[31mOturum sona erdi. Sayfayı yenile: F5\\x1b[0m\\r\\n');
    };

    term.onData((data) => ws.send(data));

    window.addEventListener('resize', () => {
      fitAddon.fit();
      ws.send(JSON.stringify({ type: 'resize', cols: term.cols, rows: term.rows }));
    });
  </script>
</body>
</html>`;

const server = http.createServer((req, res) => {
  res.writeHead(200, { 'Content-Type': 'text/html' });
  res.end(HTML);
});

const wss = new WebSocket.Server({ server, path: '/ws' });
let activeSessions = 0;

wss.on('connection', (ws) => {
  activeSessions++;
  console.log(`[+] Yeni oturum — toplam: ${activeSessions}`);

  const ptyProc = pty.spawn('moneywar-cli', [], {
    name: 'xterm-256color',
    cols: 220,
    rows: 50,
    cwd: '/app',
    env: { ...process.env, TERM: 'xterm-256color' },
  });

  ptyProc.onData((data) => {
    if (ws.readyState === WebSocket.OPEN) ws.send(data);
  });

  ptyProc.onExit(() => {
    activeSessions--;
    console.log(`[-] Oturum bitti — toplam: ${activeSessions}`);
    if (ws.readyState === WebSocket.OPEN) ws.close();
  });

  ws.on('message', (msg) => {
    try {
      const parsed = JSON.parse(msg);
      if (parsed.type === 'resize') {
        ptyProc.resize(parsed.cols, parsed.rows);
      }
    } catch {
      ptyProc.write(msg.toString());
    }
  });

  ws.on('close', () => {
    try { ptyProc.kill(); } catch {}
  });
});

const PORT = process.env.PORT || 8080;
server.listen(PORT, () => console.log(`MoneyWar session server :${PORT}`));
