import { useEffect, useState } from 'react'
import { AnimatePresence, MotionConfig, motion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { LANGS, applyLang, initialLang, setLang } from './i18n'
import logo from '../../../assets/logo.svg'
import './style.css'

const REPO = 'https://github.com/cyrill62/llegion'

const shots = import.meta.glob('../../../docs/screenshots/*.png', { eager: true, import: 'default' })
const shot = (name) => shots[`../../../docs/screenshots/${name}.png`]
const TABS = ['dashboard', 'models', 'chat', 'performance', 'gpu', 'logs', 'settings']

const fadeUp = {
  initial: { opacity: 0, y: 24 },
  whileInView: { opacity: 1, y: 0 },
  viewport: { once: true, margin: '-80px' },
  transition: { duration: 0.5, ease: 'easeOut' },
}

export default function Page() {
  const { t } = useTranslation()
  // After hydration, so the pre-rendered English page matches the first render.
  useEffect(() => applyLang(initialLang()), [])
  return (
    <MotionConfig reducedMotion="user">
      <LangMenu />
      <Hero />
      <Screens />
      <Features />
      <Terminal />
      <Proxy />
      <Download />
      <footer className="footer footer-center border-t border-base-300 p-8 text-sm">
        <p>
          {t('license')} ·{' '}
          <a className="link link-primary" href={REPO}>
            {t('source')}
          </a>
        </p>
        <p className="opacity-60">{t('disclaimer')}</p>
      </footer>
    </MotionConfig>
  )
}

const SECTION = 'mx-auto max-w-5xl px-4 py-16'

function LangMenu() {
  const { t, i18n } = useTranslation()
  return (
    <select
      aria-label={t('language')}
      className="select select-sm fixed top-4 right-4 z-10 w-auto"
      value={i18n.language}
      onChange={(e) => setLang(e.target.value)}
    >
      {LANGS.map(([code, name]) => (
        <option key={code} value={code}>
          {name}
        </option>
      ))}
    </select>
  )
}

function Hero() {
  const { t } = useTranslation()
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
          {t('heroBefore')}
          <a className="link link-primary" href="https://ollama.com">
            Ollama
          </a>
          {t('heroAfter')}
        </motion.p>
        <motion.div
          className="mt-4 flex flex-wrap justify-center gap-3"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          transition={{ delay: 0.45 }}
        >
          <a className="btn btn-primary btn-lg" href={`${REPO}/releases/latest`}>
            {t('download')}
          </a>
          <a className="btn btn-outline btn-lg" href={REPO}>
            {t('github')}
          </a>
        </motion.div>
      </div>
    </header>
  )
}

function Screens() {
  const { t } = useTranslation()
  const [tab, setTab] = useState('dashboard')
  return (
    <motion.section {...fadeUp} className="mx-auto max-w-5xl px-4">
      <div role="tablist" aria-label={t('screenshots')} className="tabs tabs-box mx-auto mb-5 w-fit flex-wrap justify-center">
        {TABS.map((name) => (
          <button
            key={name}
            role="tab"
            aria-selected={name === tab}
            className={`tab relative ${name === tab ? 'text-primary-content' : ''}`}
            onClick={() => setTab(name)}
          >
            {name === tab && <motion.span layoutId="pill" className="absolute inset-0 rounded-field bg-primary" />}
            <span className="relative">{t(`tabs.${name}`)}</span>
          </button>
        ))}
      </div>
      <div className="mockup-window border border-base-300 bg-base-200 shadow-2xl shadow-primary/30">
        <div className="aspect-[4/3] overflow-hidden">
          <AnimatePresence mode="wait">
            <motion.img
              key={tab}
              src={shot(tab)}
              alt={t('shotAlt', { tab: t(`tabs.${tab}`) })}
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
  const { t } = useTranslation()
  return (
    <section className={SECTION}>
      <motion.h2 {...fadeUp} className="mb-8 text-3xl font-bold">
        {t('featuresTitle')}
      </motion.h2>
      <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-5">
        {t('features', { returnObjects: true }).map(([title, text], i) => (
          <motion.article
            key={i}
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
  const { t } = useTranslation()
  const keys = t('terminalKeys', { returnObjects: true })
  return (
    <motion.section {...fadeUp} className={`${SECTION} grid items-center gap-10 md:grid-cols-2`}>
      <div>
        <h2 className="mb-4 text-3xl font-bold">{t('terminalTitle')}</h2>
        <p className="opacity-80">
          <code className="text-primary">llegion-tui</code>
          {t('terminalText')}
        </p>
      </div>
      <div className="mockup-code bg-black/60">
        <pre data-prefix="$">
          <code>llegion-tui</code>
        </pre>
        <pre data-prefix="#" className="opacity-60">
          <code>Tab / 1-7  {keys[0]}</code>
        </pre>
        <pre data-prefix="#" className="opacity-60">
          <code>S / R      {keys[1]}</code>
        </pre>
        <pre data-prefix="#" className="opacity-60">
          <code>P          {keys[2]}</code>
        </pre>
        <pre data-prefix="#" className="opacity-60">
          <code>?          {keys[3]}</code>
        </pre>
      </div>
    </motion.section>
  )
}

function Proxy() {
  const { t } = useTranslation()
  return (
    <motion.section {...fadeUp} className={`${SECTION} grid items-center gap-10 md:grid-cols-2`}>
      <div>
        <h2 className="mb-4 text-3xl font-bold">{t('proxyTitle')}</h2>
        <p className="opacity-80">{t('proxyText')}</p>
        <p className="mt-4 opacity-80">{t('balancingText')}</p>
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
  const { t } = useTranslation()
  return (
    <motion.section {...fadeUp} className={`${SECTION} flex flex-col items-center gap-4 text-center`}>
      <h2 className="text-3xl font-bold">{t('getTitle')}</h2>
      <p className="opacity-80">{t('getText')}</p>
      <a className="btn btn-primary btn-lg" href={`${REPO}/releases/latest`}>
        {t('latest')}
      </a>
      <p className="opacity-60">{t('fromSource')}</p>
      <div className="mockup-code bg-black/60 text-left">
        <pre data-prefix="$">
          <code>cargo build --release</code>
        </pre>
      </div>
    </motion.section>
  )
}
