import DefaultTheme from 'vitepress/theme'
import './custom.css'

export default {
  extends: DefaultTheme,
  enhanceApp({ app }) {
    // Force dark mode — no light mode toggle
    if (typeof document !== 'undefined') {
      document.documentElement.classList.add('dark')
    }
  },
}
