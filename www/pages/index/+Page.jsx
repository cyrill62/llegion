import { useState } from 'react'
import { AnimatePresence, MotionConfig, motion } from 'motion/react'
import logo from '../../../assets/logo.svg'
import './style.css'

const REPO = 'https://github.com/cyrill62/llegion'

const shots = import.meta.glob('../../../docs/screenshots/*.png', { eager: true, import: 'default' })
const shot = (name) => shots[`../../../docs/screenshots/${name}.png`]
const TABS = ['dashboard', 'models', 'chat', 'performance', 'gpu', 'logs', 'settings']

const FEATURES = [
  ['Dashboard', 'Server status and version, running models, GPU VRAM bars, CPU and RAM at a glance.'],
  ['Models', 'Pull with progress, load with a chosen context, unload, inspect and delete models.'],
  ['Chat', 'Streamed Markdown answers, tok/s and time to first token, side-by-side regenerations to compare models.'],
  ['Performance', 'Live CPU, RAM, GPU and VRAM charts, plus a tokens/s benchmark using Ollama’s own counters.'],
  ['GPU', 'Detects NVIDIA, AMD, Intel and Apple GPUs. Auto, custom device subset or CPU only, Vulkan toggle.'],
  ['Logs', 'Live stream from the managed server, server.log or journalctl, with filters, search and save.'],
  ['HTTPS + tokens', 'A supervised Caddy proxy terminates TLS and rejects requests without a valid bearer token.'],
  ['Managed server', 'Runs ollama serve with your settings, captures its logs and restarts it if it crashes.'],
]

const fadeUp = {
  initial: { opacity: 0, y: 24 },
  whileInView: { opacity: 1, y: 0 },
  viewport: { once: true, margin: '-80px' },
  transition: { duration: 0.5, ease: 'easeOut' },
}

export default function Page() {
  return (
    <MotionConfig reducedMotion="user">
      <Hero />
      <Screens />
      <Features />
      <Terminal />
      <Proxy />
      <Download />
      <footer>
        <p>
          MIT licensed · <a href={REPO}>Source on GitHub</a>
        </p>
        <p className="muted">Llegion is an independent community project, not affiliated with or endorsed by Ollama.</p>
      </footer>
    </MotionConfig>
  )
}

function Hero() {
  return (
    <header className="hero">
      <motion.img
        src={logo}
        alt=""
        className="logo"
        initial={{ opacity: 0, scale: 0.8, rotate: -8 }}
        animate={{ opacity: 1, scale: 1, rotate: 0 }}
        transition={{ type: 'spring', stiffness: 120, damping: 14 }}
      />
      <motion.h1 initial={{ opacity: 0, y: 16 }} animate={{ opacity: 1, y: 0 }} transition={{ delay: 0.15 }}>
        Llegion
      </motion.h1>
      <motion.p
        className="tagline"
        initial={{ opacity: 0, y: 16 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ delay: 0.3 }}
      >
        A fast, native control panel for <a href="https://ollama.com">Ollama</a>. Run, monitor, benchmark and secure your
        local models from a desktop app or a terminal UI, on Linux, Windows and macOS.
      </motion.p>
      <motion.div className="cta" initial={{ opacity: 0 }} animate={{ opacity: 1 }} transition={{ delay: 0.45 }}>
        <a className="btn primary" href={`${REPO}/releases/latest`}>
          Download
        </a>
        <a className="btn" href={REPO}>
          View on GitHub
        </a>
      </motion.div>
    </header>
  )
}

function Screens() {
  const [tab, setTab] = useState('dashboard')
  return (
    <motion.section {...fadeUp} className="screens">
      <div className="tabs" role="tablist" aria-label="Screenshots">
        {TABS.map((t) => (
          <button key={t} role="tab" aria-selected={t === tab} onClick={() => setTab(t)}>
            {t === tab && <motion.span layoutId="pill" className="pill" />}
            <span>{t}</span>
          </button>
        ))}
      </div>
      <div className="frame">
        <AnimatePresence mode="wait">
          <motion.img
            key={tab}
            src={shot(tab)}
            alt={`Llegion ${tab} tab`}
            width="1440"
            height="1080"
            initial={{ opacity: 0, scale: 0.98 }}
            animate={{ opacity: 1, scale: 1 }}
            exit={{ opacity: 0, scale: 0.98 }}
            transition={{ duration: 0.25 }}
          />
        </AnimatePresence>
      </div>
    </motion.section>
  )
}

function Features() {
  return (
    <section>
      <motion.h2 {...fadeUp}>Everything Ollama, in one place</motion.h2>
      <div className="grid">
        {FEATURES.map(([title, text], i) => (
          <motion.article
            key={title}
            className="card"
            {...fadeUp}
            transition={{ ...fadeUp.transition, delay: (i % 4) * 0.08 }}
            whileHover={{ y: -4 }}
          >
            <h3>{title}</h3>
            <p>{text}</p>
          </motion.article>
        ))}
      </div>
    </section>
  )
}

function Terminal() {
  return (
    <motion.section {...fadeUp} className="split">
      <div>
        <h2>Works over SSH too</h2>
        <p>
          <code>llegion-tui</code> brings the same seven tabs to the terminal, on top of the same core: settings, managed
          server, HTTPS proxy, logs and benchmarks. Keyboard first, mouse friendly, no GUI libraries required.
        </p>
      </div>
      <pre className="term">
        <span className="muted">$ </span>llegion-tui{'\n'}
        <span className="muted"># Tab / 1-7  switch tabs</span>
        {'\n'}
        <span className="muted"># S / R      start-stop / restart server</span>
        {'\n'}
        <span className="muted"># P          toggle the HTTPS proxy</span>
        {'\n'}
        <span className="muted"># ?          all keys</span>
      </pre>
    </motion.section>
  )
}

function Proxy() {
  return (
    <motion.section {...fadeUp} className="split">
      <div>
        <h2>Expose Ollama safely</h2>
        <p>
          Ollama has no authentication. Turn on the proxy and Llegion binds Ollama to loopback, generates a Caddyfile and
          supervises Caddy: TLS (self-signed, Let’s Encrypt or your own cert), bearer tokens you can label, copy and
          revoke, optional CORS, and streamed tokens that arrive immediately.
        </p>
      </div>
      <pre className="term">
        <span className="muted">$ </span>curl https://my-box:8443/api/tags{'\n'}
        <span className="err">401 Unauthorized</span>
        {'\n\n'}
        <span className="muted">$ </span>curl -H "Authorization: Bearer $TOKEN" \{'\n'}
        {'    '}https://my-box:8443/api/tags{'\n'}
        <span className="ok">200 OK</span>
      </pre>
    </motion.section>
  )
}

function Download() {
  return (
    <motion.section {...fadeUp} className="download">
      <h2>Get Llegion</h2>
      <p>Prebuilt archives for Linux (x64, arm64, glibc and musl), macOS (universal .app) and Windows (x64, arm64).</p>
      <a className="btn primary" href={`${REPO}/releases/latest`}>
        Latest release
      </a>
      <p className="muted">Or build from source:</p>
      <pre className="term inline">cargo build --release</pre>
    </motion.section>
  )
}
