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
      <footer className="footer footer-center border-t border-base-300 p-8 text-sm">
        <p>
          MIT licensed ·{' '}
          <a className="link link-primary" href={REPO}>
            Source on GitHub
          </a>
        </p>
        <p className="opacity-60">Llegion is an independent community project, not affiliated with or endorsed by Ollama.</p>
      </footer>
    </MotionConfig>
  )
}

const SECTION = 'mx-auto max-w-5xl px-4 py-16'

function Hero() {
  return (
    <header className="hero pt-24 pb-16">
      <div className="hero-content flex-col text-center">
        <motion.img
          src={logo}
          alt=""
          className="w-28"
          initial={{ opacity: 0, scale: 0.8, rotate: -8 }}
          animate={{ opacity: 1, scale: 1, rotate: 0 }}
          transition={{ type: 'spring', stiffness: 120, damping: 14 }}
        />
        <motion.h1
          className="text-5xl font-bold tracking-tight sm:text-7xl"
          initial={{ opacity: 0, y: 16 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ delay: 0.15 }}
        >
          Llegion
        </motion.h1>
        <motion.p
          className="max-w-xl text-lg opacity-70"
          initial={{ opacity: 0, y: 16 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ delay: 0.3 }}
        >
          A fast, native control panel for{' '}
          <a className="link link-primary" href="https://ollama.com">
            Ollama
          </a>
          . Run, monitor, benchmark and secure your local models from a desktop app or a terminal UI, on Linux, Windows
          and macOS.
        </motion.p>
        <motion.div
          className="mt-4 flex flex-wrap justify-center gap-3"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          transition={{ delay: 0.45 }}
        >
          <a className="btn btn-primary btn-lg" href={`${REPO}/releases/latest`}>
            Download
          </a>
          <a className="btn btn-outline btn-lg" href={REPO}>
            View on GitHub
          </a>
        </motion.div>
      </div>
    </header>
  )
}

function Screens() {
  const [tab, setTab] = useState('dashboard')
  return (
    <motion.section {...fadeUp} className="mx-auto max-w-5xl px-4">
      <div role="tablist" aria-label="Screenshots" className="tabs tabs-box mx-auto mb-5 w-fit flex-wrap justify-center">
        {TABS.map((t) => (
          <button
            key={t}
            role="tab"
            aria-selected={t === tab}
            className={`tab relative capitalize ${t === tab ? 'text-primary-content' : ''}`}
            onClick={() => setTab(t)}
          >
            {t === tab && <motion.span layoutId="pill" className="absolute inset-0 rounded-field bg-primary" />}
            <span className="relative">{t}</span>
          </button>
        ))}
      </div>
      <div className="mockup-window border border-base-300 bg-base-200 shadow-2xl shadow-primary/30">
        <div className="aspect-[4/3] overflow-hidden">
          <AnimatePresence mode="wait">
            <motion.img
              key={tab}
              src={shot(tab)}
              alt={`Llegion ${tab} tab`}
              width="1440"
              height="1080"
              className="block w-full"
              initial={{ opacity: 0, scale: 0.98 }}
              animate={{ opacity: 1, scale: 1 }}
              exit={{ opacity: 0, scale: 0.98 }}
              transition={{ duration: 0.25 }}
            />
          </AnimatePresence>
        </div>
      </div>
    </motion.section>
  )
}

function Features() {
  return (
    <section className={SECTION}>
      <motion.h2 {...fadeUp} className="mb-8 text-3xl font-bold">
        Everything Ollama, in one place
      </motion.h2>
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
        {FEATURES.map(([title, text], i) => (
          <motion.article
            key={title}
            className="card border border-base-300 bg-base-200"
            {...fadeUp}
            transition={{ ...fadeUp.transition, delay: (i % 4) * 0.08 }}
            whileHover={{ y: -4 }}
          >
            <div className="card-body">
              <h3 className="card-title text-primary">{title}</h3>
              <p className="text-sm opacity-70">{text}</p>
            </div>
          </motion.article>
        ))}
      </div>
    </section>
  )
}

function Terminal() {
  return (
    <motion.section {...fadeUp} className={`${SECTION} grid items-center gap-10 md:grid-cols-2`}>
      <div>
        <h2 className="mb-4 text-3xl font-bold">Works over SSH too</h2>
        <p className="opacity-80">
          <code className="text-primary">llegion-tui</code> brings the same seven tabs to the terminal, on top of the same
          core: settings, managed server, HTTPS proxy, logs and benchmarks. Keyboard first, mouse friendly, no GUI
          libraries required.
        </p>
      </div>
      <div className="mockup-code bg-black/60">
        <pre data-prefix="$">
          <code>llegion-tui</code>
        </pre>
        <pre data-prefix="#" className="opacity-60">
          <code>Tab / 1-7  switch tabs</code>
        </pre>
        <pre data-prefix="#" className="opacity-60">
          <code>S / R      start-stop / restart server</code>
        </pre>
        <pre data-prefix="#" className="opacity-60">
          <code>P          toggle the HTTPS proxy</code>
        </pre>
        <pre data-prefix="#" className="opacity-60">
          <code>?          all keys</code>
        </pre>
      </div>
    </motion.section>
  )
}

function Proxy() {
  return (
    <motion.section {...fadeUp} className={`${SECTION} grid items-center gap-10 md:grid-cols-2`}>
      <div>
        <h2 className="mb-4 text-3xl font-bold">Expose Ollama safely</h2>
        <p className="opacity-80">
          Ollama has no authentication. Turn on the proxy and Llegion binds Ollama to loopback, generates a Caddyfile and
          supervises Caddy: TLS (self-signed, Let’s Encrypt or your own cert), bearer tokens you can label, copy and
          revoke, optional CORS, and streamed tokens that arrive immediately.
        </p>
      </div>
      <div className="mockup-code bg-black/60">
        <pre data-prefix="$">
          <code>curl https://my-box:8443/api/tags</code>
        </pre>
        <pre data-prefix=">" className="text-error">
          <code>401 Unauthorized</code>
        </pre>
        <pre data-prefix="$">
          <code>curl -H "Authorization: Bearer $TOKEN" \</code>
        </pre>
        <pre data-prefix="">
          <code>{'    '}https://my-box:8443/api/tags</code>
        </pre>
        <pre data-prefix=">" className="text-success">
          <code>200 OK</code>
        </pre>
      </div>
    </motion.section>
  )
}

function Download() {
  return (
    <motion.section {...fadeUp} className={`${SECTION} flex flex-col items-center gap-4 text-center`}>
      <h2 className="text-3xl font-bold">Get Llegion</h2>
      <p className="opacity-80">
        Prebuilt archives for Linux (x64, arm64, glibc and musl), macOS (universal .app) and Windows (x64, arm64).
      </p>
      <a className="btn btn-primary btn-lg" href={`${REPO}/releases/latest`}>
        Latest release
      </a>
      <p className="opacity-60">Or build from source:</p>
      <div className="mockup-code bg-black/60 text-left">
        <pre data-prefix="$">
          <code>cargo build --release</code>
        </pre>
      </div>
    </motion.section>
  )
}
