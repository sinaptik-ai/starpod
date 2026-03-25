import { defineConfig } from 'vitepress'

export default defineConfig({
  title: 'Starpod',
  description: 'A local-first personal AI assistant platform built in Rust',
  base: process.env.DOCS_BASE || '/docs/',
  head: [
    ['link', { rel: 'icon', type: 'image/svg+xml', href: `${process.env.DOCS_BASE || '/docs/'}favicon.svg` }],
    ['meta', { name: 'theme-color', content: '#0A0A0A' }],
  ],
  appearance: 'dark',
  cleanUrls: true,
  lastUpdated: true,
  ignoreDeadLinks: [
    /localhost/,
  ],

  themeConfig: {
    logo: '/logo.svg',
    siteTitle: 'Starpod',

    nav: [
      { text: 'Guide', link: '/getting-started/installation' },
      { text: 'Concepts', link: '/concepts/memory' },
      { text: 'API', link: '/api-reference/overview' },
      { text: 'Crates', link: '/crates/agent-sdk' },
    ],

    sidebar: {
      '/': [
        {
          text: 'Introduction',
          items: [
            { text: 'What is Starpod?', link: '/' },
            { text: 'Architecture', link: '/architecture' },
          ],
        },
        {
          text: 'Getting Started',
          items: [
            { text: 'Installation', link: '/getting-started/installation' },
            { text: 'Project Setup', link: '/getting-started/initialization' },
            { text: 'Configuration', link: '/getting-started/configuration' },
            { text: 'Quick Start', link: '/getting-started/quickstart' },
          ],
        },
        {
          text: 'Core Concepts',
          items: [
            { text: 'Memory', link: '/concepts/memory' },
            { text: 'Sessions', link: '/concepts/sessions' },
            { text: 'Skills', link: '/concepts/skills' },
            { text: 'Cron & Scheduling', link: '/concepts/cron' },
            { text: 'Lifecycle Prompts', link: '/concepts/heartbeat' },
            { text: 'Vault', link: '/concepts/vault' },
            { text: 'Agent Tools', link: '/concepts/tools' },
            { text: 'Instances', link: '/concepts/instances' },
          ],
        },
        {
          text: 'Integrations',
          items: [
            { text: 'Web UI', link: '/integrations/web-ui' },
            { text: 'Telegram Bot', link: '/integrations/telegram' },
            { text: 'WebSocket', link: '/integrations/websocket' },
          ],
        },
        {
          text: 'API Reference',
          items: [
            { text: 'Overview', link: '/api-reference/overview' },
            { text: 'Chat', link: '/api-reference/chat' },
            { text: 'Sessions', link: '/api-reference/sessions' },
            { text: 'Memory', link: '/api-reference/memory' },
            { text: 'Files', link: '/api-reference/files' },
            { text: 'Health', link: '/api-reference/health' },
            { text: 'Instances', link: '/api-reference/instances' },
          ],
        },
        {
          text: 'CLI Reference',
          link: '/cli-reference',
        },
        {
          text: 'Crate Reference',
          collapsed: true,
          items: [
            { text: 'agent-sdk', link: '/crates/agent-sdk' },
            { text: 'starpod-hooks', link: '/crates/starpod-hooks' },
            { text: 'starpod-core', link: '/crates/starpod-core' },
            { text: 'starpod-memory', link: '/crates/starpod-memory' },
            { text: 'starpod-vault', link: '/crates/starpod-vault' },
            { text: 'starpod-session', link: '/crates/starpod-session' },
            { text: 'starpod-skills', link: '/crates/starpod-skills' },
            { text: 'starpod-cron', link: '/crates/starpod-cron' },
            { text: 'starpod-agent', link: '/crates/starpod-agent' },
            { text: 'starpod-gateway', link: '/crates/starpod-gateway' },
            { text: 'starpod-telegram', link: '/crates/starpod-telegram' },
            { text: 'starpod-instances', link: '/crates/starpod-instances' },
          ],
        },
      ],
    },

    socialLinks: [
      { icon: 'github', link: 'https://github.com/sinaptik-ai/starpod' },
      { icon: 'discord', link: 'https://discord.com/invite/KYKj9F2FRH' },
    ],

    search: {
      provider: 'local',
    },

    editLink: {
      pattern: 'https://github.com/sinaptik-ai/starpod/edit/main/docs/:path',
      text: 'Edit this page on GitHub',
    },

    footer: {
      message: 'Released under the MIT License.',
      copyright: 'Built with Rust and Claude.',
    },
  },

  markdown: {
    lineNumbers: true,
  },
})
