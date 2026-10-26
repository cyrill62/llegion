import tailwindcss from '@tailwindcss/vite'
import react from '@vitejs/plugin-react'
import vike from 'vike/plugin'
import { defineConfig } from 'vite'

export default defineConfig({
  // GitHub Pages project site: https://cyrill62.github.io/llegion/
  base: '/llegion/',
  plugins: [vike(), react(), tailwindcss()],
  // The logo and screenshots are imported from the repo root, so they stay in one place.
  server: { fs: { allow: ['..'] } },
})
