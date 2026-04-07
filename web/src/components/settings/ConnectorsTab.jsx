import { useState, useEffect, useCallback, useMemo } from "react";
import { apiHeaders } from "../../lib/api";
import { Loading } from "../ui/EmptyState";
import SlackSetupGuide from "./SlackSetupGuide";

// ── Brand logos (filled SVGs, 24x24) ──────────────────────────────────────

const LOGOS = {
  github: (
    <svg width="24" height="24" viewBox="0 0 24 24" fill="currentColor">
      <path d="M12 0C5.37 0 0 5.37 0 12c0 5.31 3.435 9.795 8.205 11.385.6.105.825-.255.825-.57 0-.285-.015-1.23-.015-2.235-3.015.555-3.795-.735-4.035-1.41-.135-.345-.72-1.41-1.23-1.695-.42-.225-1.02-.78-.015-.795.945-.015 1.62.87 1.845 1.23 1.08 1.815 2.805 1.305 3.495.99.105-.78.42-1.305.765-1.605-2.67-.3-5.46-1.335-5.46-5.925 0-1.305.465-2.385 1.23-3.225-.12-.3-.54-1.53.12-3.18 0 0 1.005-.315 3.3 1.23.96-.27 1.98-.405 3-.405s2.04.135 3 .405c2.295-1.56 3.3-1.23 3.3-1.23.66 1.65.24 2.88.12 3.18.765.84 1.23 1.905 1.23 3.225 0 4.605-2.805 5.625-5.475 5.925.435.375.81 1.095.81 2.22 0 1.605-.015 2.895-.015 3.3 0 .315.225.69.825.57A12.02 12.02 0 0 0 24 12c0-6.63-5.37-12-12-12z" />
    </svg>
  ),
  "github-apps": (
    <svg width="24" height="24" viewBox="0 0 24 24" fill="currentColor">
      <path d="M12 0C5.37 0 0 5.37 0 12c0 5.31 3.435 9.795 8.205 11.385.6.105.825-.255.825-.57 0-.285-.015-1.23-.015-2.235-3.015.555-3.795-.735-4.035-1.41-.135-.345-.72-1.41-1.23-1.695-.42-.225-1.02-.78-.015-.795.945-.015 1.62.87 1.845 1.23 1.08 1.815 2.805 1.305 3.495.99.105-.78.42-1.305.765-1.605-2.67-.3-5.46-1.335-5.46-5.925 0-1.305.465-2.385 1.23-3.225-.12-.3-.54-1.53.12-3.18 0 0 1.005-.315 3.3 1.23.96-.27 1.98-.405 3-.405s2.04.135 3 .405c2.295-1.56 3.3-1.23 3.3-1.23.66 1.65.24 2.88.12 3.18.765.84 1.23 1.905 1.23 3.225 0 4.605-2.805 5.625-5.475 5.925.435.375.81 1.095.81 2.22 0 1.605-.015 2.895-.015 3.3 0 .315.225.69.825.57A12.02 12.02 0 0 0 24 12c0-6.63-5.37-12-12-12z" />
    </svg>
  ),
  postgres: (
    <svg
      width="24"
      height="24"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
    >
      <ellipse cx="12" cy="6" rx="8" ry="3" />
      <path d="M4 6v6c0 1.66 3.58 3 8 3s8-1.34 8-3V6" />
      <path d="M4 12v6c0 1.66 3.58 3 8 3s8-1.34 8-3v-6" />
    </svg>
  ),
  mysql: (
    <svg
      width="24"
      height="24"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
    >
      <ellipse cx="12" cy="6" rx="8" ry="3" />
      <path d="M4 6v6c0 1.66 3.58 3 8 3s8-1.34 8-3V6" />
      <path d="M4 12v6c0 1.66 3.58 3 8 3s8-1.34 8-3v-6" />
      <line x1="20" y1="6" x2="20" y2="18" strokeDasharray="2 2" />
    </svg>
  ),
  redis: (
    <svg
      width="24"
      height="24"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
    >
      <path d="M12 3L2 8l10 5 10-5-10-5z" />
      <path d="M2 13l10 5 10-5" />
      <path d="M2 18l10 5 10-5" />
    </svg>
  ),
  mongodb: (
    <svg width="24" height="24" viewBox="0 0 24 24" fill="currentColor">
      <path d="M17.193 9.555c-1.264-5.58-4.252-7.414-4.573-8.115-.28-.394-.53-.954-.735-1.44-.036.495-.055.685-.523 1.184-.723.566-4.438 3.682-4.74 10.02-.282 5.912 4.27 9.435 4.888 9.884l.07.05A73.49 73.49 0 0 1 11.91 24h.19c.28-1.28.49-2.26.676-3.27.418-.225 3.24-1.97 4.417-7.175z" />
      <path
        d="M12.61 24c.08-.42.189-.84.31-1.25.055-.02.11-.06.165-.1-.06.04-.11.08-.165.1.13-.39.275-.78.44-1.17-.28-.155-.5-.31-.69-.5l-.07-.05c-.06.39-.12.79-.17 1.18-.11.56-.22 1.15-.34 1.79h.52z"
        opacity=".5"
      />
    </svg>
  ),
  elasticsearch: (
    <svg width="24" height="24" viewBox="0 0 24 24" fill="currentColor">
      <path d="M13.394 0C8.217 0 3.82 3.38 2.186 8h14.808a3 3 0 0 0 2.486-1.325l2.16-3.2C19.677 1.32 16.746 0 13.394 0z" />
      <path
        d="M1.549 10C1.2 10.955 1 11.955 1 13c0 1.045.2 2.045.549 3h12.257a2 2 0 0 0 1.657-.89l2.758-4.11a2 2 0 0 0 0-2.22l-2.148-3.2A2 2 0 0 0 14.416 10H1.549z"
        opacity=".7"
      />
      <path
        d="M2.186 18c1.634 4.62 6.03 8 11.208 8 3.352 0 6.283-1.32 8.246-3.475l-2.16-3.2A3 3 0 0 0 16.994 18H2.186z"
        opacity=".4"
      />
    </svg>
  ),
  stripe: (
    <svg width="24" height="24" viewBox="0 0 24 24" fill="currentColor">
      <path d="M13.976 9.15c-2.172-.806-3.356-1.426-3.356-2.409 0-.831.683-1.305 1.901-1.305 2.227 0 4.515.858 6.09 1.631l.89-5.494C18.252.975 15.697 0 12.165 0 9.667 0 7.589.654 6.104 1.872 4.56 3.147 3.757 4.918 3.757 7.076c0 4.72 2.891 6.394 6.022 7.593 2.018.767 2.964 1.416 2.964 2.345 0 .932-.784 1.478-2.228 1.478-1.91 0-4.85-.971-6.756-2.121L3 21.97C5.12 23.265 8.122 24 10.857 24c2.618 0 4.772-.64 6.265-1.874 1.602-1.326 2.405-3.244 2.405-5.532 0-4.858-2.962-6.528-5.551-7.444z" />
    </svg>
  ),
  aws: (
    <svg width="24" height="24" viewBox="0 0 24 24" fill="currentColor">
      <path
        d="M6.763 10.036a7.61 7.61 0 0 0 .065 1.396c.073.38.182.782.327 1.16.054.127.073.254.073.363 0 .163-.109.327-.309.49l-1.026.69a.773.773 0 0 1-.418.145c-.163 0-.327-.072-.472-.218a4.84 4.84 0 0 1-.563-.735 12.59 12.59 0 0 1-.49-.926c-1.228 1.447-2.77 2.17-4.625 2.17-1.32 0-2.373-.381-3.152-1.133C-4.604 12.71-5 11.756-5 10.583c0-1.247.436-2.261 1.32-3.03.884-.77 2.062-1.153 3.553-1.153.49 0 1.008.036 1.554.109.545.072 1.108.182 1.699.327V5.78c0-1.136-.236-1.934-.727-2.388C1.908 2.93 1.09 2.71 0 2.71a8.33 8.33 0 0 0-1.87.218c-.635.145-1.252.345-1.843.581a4.396 4.396 0 0 1-.545.218.87.87 0 0 1-.236.036c-.218 0-.327-.163-.327-.49V2.3c0-.254.036-.436.127-.545A1.34 1.34 0 0 1-4.222 1.5c.6-.29 1.3-.536 2.098-.727C-1.318.59-.474.5.387.5c1.79 0 3.097.418 3.935 1.247.83.83 1.247 2.098 1.247 3.79v4.499h1.194zm-6.39 2.388c.472 0 .963-.09 1.481-.254.518-.163.98-.472 1.372-.89.236-.272.418-.58.509-.944.09-.363.145-.8.145-1.3v-.626a12.52 12.52 0 0 0-1.355-.254 11.35 11.35 0 0 0-1.39-.09c-.963 0-1.664.182-2.134.563-.472.381-.7.917-.7 1.627 0 .654.163 1.153.509 1.481.327.345.8.509 1.39.509l.173.178z"
        transform="translate(5.5 4)"
      />
      <path
        d="M18.5 16.8c-2.654 1.953-6.5 2.993-9.812 2.993-4.643 0-8.824-1.717-11.988-4.57-.254-.218-.027-.527.272-.363 3.408 1.99 7.625 3.187 11.988 3.187 2.94 0 6.17-.617 9.139-1.88.454-.182.826.29.4.633z"
        opacity=".7"
      />
      <path
        d="M19.7 15.4c-.345-.436-2.28-.218-3.152-.109-.254.036-.29-.2-.063-.363 1.554-1.09 4.088-.772 4.387-.4.29.363-.09 2.934-1.536 4.16-.218.182-.436.09-.327-.163.327-.8 1.036-2.689.69-3.125z"
        opacity=".7"
      />
    </svg>
  ),
  "google-cloud": (
    <svg width="24" height="24" viewBox="0 0 24 24" fill="currentColor">
      <path
        d="M12.19 2a9.344 9.344 0 0 0-8.17 4.806l3.06 2.392A5.563 5.563 0 0 1 12.19 5.8h.01a5.518 5.518 0 0 1 3.874 1.593l2.8-2.8A9.325 9.325 0 0 0 12.19 2z"
        opacity=".8"
      />
      <path
        d="M21.5 12.19c0-.69-.068-1.361-.19-2.01h-5.7v4h5.07a5.545 5.545 0 0 1-1.895 2.845L21.5 12.19z"
        opacity=".6"
      />
      <path
        d="M5.02 14.26A5.569 5.569 0 0 1 6.6 12.19c0-1.03.282-1.993.768-2.822L4.02 6.806A9.319 9.319 0 0 0 2.85 12.19c0 1.877.553 3.623 1.498 5.09l2.672-3.02z"
        opacity=".4"
      />
      <path
        d="M12.19 22c2.509 0 4.808-.854 6.59-2.3l-3.08-2.595a5.573 5.573 0 0 1-3.51 1.095 5.563 5.563 0 0 1-5.119-3.94L4.02 17.28A9.33 9.33 0 0 0 12.19 22z"
        opacity=".6"
      />
    </svg>
  ),
  azure: (
    <svg width="24" height="24" viewBox="0 0 24 24" fill="currentColor">
      <path d="M13.05 4.24L7.56 18.5l-4.82 0 5.49-14.26H13.05z" opacity=".8" />
      <path d="M20.44 17.34l-8.3 1.78L7.56 18.5l4.22-4.92 2.49-6.64L20.44 17.34z" />
    </svg>
  ),
  slack: (
    <svg width="24" height="24" viewBox="0 0 24 24" fill="currentColor">
      <path d="M5.042 15.165a2.528 2.528 0 0 1-2.52 2.523A2.528 2.528 0 0 1 0 15.165a2.527 2.527 0 0 1 2.522-2.52h2.52v2.52zm1.271 0a2.527 2.527 0 0 1 2.521-2.52 2.527 2.527 0 0 1 2.521 2.52v6.313A2.528 2.528 0 0 1 8.834 24a2.528 2.528 0 0 1-2.521-2.522v-6.313zM8.834 5.042a2.528 2.528 0 0 1-2.521-2.52A2.528 2.528 0 0 1 8.834 0a2.528 2.528 0 0 1 2.521 2.522v2.52H8.834zm0 1.271a2.528 2.528 0 0 1 2.521 2.521 2.528 2.528 0 0 1-2.521 2.521H2.522A2.528 2.528 0 0 1 0 8.834a2.528 2.528 0 0 1 2.522-2.521h6.312zm10.122 2.521a2.528 2.528 0 0 1 2.522-2.521A2.528 2.528 0 0 1 24 8.834a2.528 2.528 0 0 1-2.522 2.521h-2.522V8.834zm-1.268 0a2.528 2.528 0 0 1-2.523 2.521 2.527 2.527 0 0 1-2.52-2.521V2.522A2.527 2.527 0 0 1 15.165 0a2.528 2.528 0 0 1 2.523 2.522v6.312zm-2.523 10.122a2.528 2.528 0 0 1 2.523 2.522A2.528 2.528 0 0 1 15.165 24a2.527 2.527 0 0 1-2.52-2.522v-2.522h2.52zm0-1.268a2.527 2.527 0 0 1-2.52-2.523 2.526 2.526 0 0 1 2.52-2.52h6.313A2.527 2.527 0 0 1 24 15.165a2.528 2.528 0 0 1-2.522 2.523h-6.313z" />
    </svg>
  ),
  discord: (
    <svg width="24" height="24" viewBox="0 0 24 24" fill="currentColor">
      <path d="M20.317 4.37a19.791 19.791 0 0 0-4.885-1.515.074.074 0 0 0-.079.037c-.21.375-.444.864-.608 1.25a18.27 18.27 0 0 0-5.487 0 12.64 12.64 0 0 0-.617-1.25.077.077 0 0 0-.079-.037A19.736 19.736 0 0 0 3.677 4.37a.07.07 0 0 0-.032.027C.533 9.046-.32 13.58.099 18.057a.082.082 0 0 0 .031.057 19.9 19.9 0 0 0 5.993 3.03.078.078 0 0 0 .084-.028c.462-.63.874-1.295 1.226-1.994a.076.076 0 0 0-.041-.106 13.107 13.107 0 0 1-1.872-.892.077.077 0 0 1-.008-.128 10.2 10.2 0 0 0 .372-.292.074.074 0 0 1 .077-.01c3.928 1.793 8.18 1.793 12.062 0a.074.074 0 0 1 .078.01c.12.098.246.198.373.292a.077.077 0 0 1-.006.127 12.299 12.299 0 0 1-1.873.892.077.077 0 0 0-.041.107c.36.698.772 1.362 1.225 1.993a.076.076 0 0 0 .084.028 19.839 19.839 0 0 0 6.002-3.03.077.077 0 0 0 .032-.054c.5-5.177-.838-9.674-3.549-13.66a.061.061 0 0 0-.031-.03zM8.02 15.33c-1.183 0-2.157-1.085-2.157-2.419 0-1.333.956-2.419 2.157-2.419 1.21 0 2.176 1.096 2.157 2.42 0 1.333-.956 2.418-2.157 2.418zm7.975 0c-1.183 0-2.157-1.085-2.157-2.419 0-1.333.955-2.419 2.157-2.419 1.21 0 2.176 1.096 2.157 2.42 0 1.333-.946 2.418-2.157 2.418z" />
    </svg>
  ),
  "google-calendar": (
    <svg
      width="24"
      height="24"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <rect x="3" y="4" width="18" height="18" rx="2" />
      <line x1="16" y1="2" x2="16" y2="6" />
      <line x1="8" y1="2" x2="8" y2="6" />
      <line x1="3" y1="10" x2="21" y2="10" />
      <path d="M10 14l2 2 4-4" />
    </svg>
  ),
  "google-sheets": (
    <svg width="24" height="24" viewBox="0 0 24 24" fill="currentColor">
      <path
        d="M14.727 6.727H14V0H4.91c-.905 0-1.637.732-1.637 1.636v20.728c0 .904.732 1.636 1.636 1.636h14.182c.904 0 1.636-.732 1.636-1.636V6.727h-5.273z"
        opacity=".4"
      />
      <path d="M14.727 0v6.727h6L14.727 0z" opacity=".6" />
      <path d="M7.5 12h9v1.5h-9V12zm0 3h9v1.5h-9V15zm0 3h6v1.5h-6V18z" />
    </svg>
  ),
  "meta-ads": (
    <svg width="24" height="24" viewBox="0 0 24 24" fill="currentColor">
      <path d="M12 2.04c-5.5 0-10 4.49-10 10.02 0 5 3.66 9.15 8.44 9.9v-7H7.9v-2.9h2.54V9.85c0-2.51 1.49-3.89 3.78-3.89 1.09 0 2.24.19 2.24.19v2.47h-1.26c-1.24 0-1.63.77-1.63 1.56v1.88h2.78l-.45 2.9h-2.33v7a10 10 0 0 0 8.44-9.9c0-5.53-4.5-10.02-10.01-10.02z" />
    </svg>
  ),
  linear: (
    <svg width="24" height="24" viewBox="0 0 24 24" fill="currentColor">
      <path d="M3.357 7.07a10.025 10.025 0 0 0-.357 2.636c0 5.523 4.477 10 10 10 .924 0 1.82-.125 2.67-.36L3.357 7.07zm1.752-2.563l14.35 14.392A9.968 9.968 0 0 0 23 12c0-5.523-4.477-10-10-10a9.968 9.968 0 0 0-7.891 3.507z" />
    </svg>
  ),
  smtp: (
    <svg
      width="24"
      height="24"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <rect x="2" y="4" width="20" height="16" rx="2" />
      <polyline points="22,6 12,13 2,6" />
    </svg>
  ),
  telegram: (
    <svg width="24" height="24" viewBox="0 0 24 24" fill="currentColor">
      <path d="M11.944 0A12 12 0 0 0 0 12a12 12 0 0 0 12 12 12 12 0 0 0 12-12A12 12 0 0 0 12 0h-.056zm4.962 7.224c.1-.002.321.023.465.14a.506.506 0 0 1 .171.325c.016.093.036.306.02.472-.18 1.898-.962 6.502-1.36 8.627-.168.9-.499 1.201-.82 1.23-.696.065-1.225-.46-1.9-.902-1.056-.693-1.653-1.124-2.678-1.8-1.185-.78-.417-1.21.258-1.91.177-.184 3.247-2.977 3.307-3.23.007-.032.014-.15-.056-.212s-.174-.041-.249-.024c-.106.024-1.793 1.14-5.061 3.345-.479.33-.913.49-1.302.48-.428-.008-1.252-.241-1.865-.44-.752-.245-1.349-.374-1.297-.789.027-.216.325-.437.893-.663 3.498-1.524 5.83-2.529 6.998-3.014 3.332-1.386 4.025-1.627 4.476-1.635z" />
    </svg>
  ),
  cloudflare: (
    <svg width="24" height="24" viewBox="0 0 24 24" fill="currentColor">
      <path d="M16.51 17.93l.32-1.11a1.37 1.37 0 0 0-.06-.98 1.12 1.12 0 0 0-.77-.58l-8.79-.3a.24.24 0 0 1-.2-.12.23.23 0 0 1 0-.23.29.29 0 0 1 .26-.18l8.93-.3a2.54 2.54 0 0 0 2.15-1.78l.55-1.89a.36.36 0 0 0 .01-.14 5.73 5.73 0 0 0-11.05-1.05 3.65 3.65 0 0 0-5.7 3.61A4.59 4.59 0 0 0 0 18.75a.33.33 0 0 0 .33.27h15.86a.34.34 0 0 0 .32-.24z" />
      <path d="M19.35 11.06a.12.12 0 0 0-.12 0 .1.1 0 0 0-.06.1l-.17.6a1.37 1.37 0 0 1-.06.98 1.12 1.12 0 0 1-.77.58l-1.6.06a.24.24 0 0 0-.2.12.23.23 0 0 0 0 .23.29.29 0 0 0 .26.18l1.74.06a2.54 2.54 0 0 1 2.15 1.78l.15.52a.14.14 0 0 0 .13.1 4.14 4.14 0 0 0-1.45-5.31z" />
    </svg>
  ),
  notion: (
    <svg width="24" height="24" viewBox="0 0 24 24" fill="currentColor">
      <path
        d="M4.459 4.208c.746.606 1.026.56 2.428.466l13.215-.793c.28 0 .047-.28-.046-.326L18.09 2.35c-.42-.326-.98-.7-2.054-.607l-12.79.933c-.467.047-.56.28-.374.466l1.587 1.066zm.793 3.08v13.904c0 .747.373 1.027 1.214.98l14.523-.84c.841-.046.934-.56.934-1.166V6.354c0-.606-.233-.933-.748-.886l-15.177.84c-.56.047-.746.327-.746.98zm14.337.745c.093.42 0 .84-.42.888l-.7.14v10.264c-.608.327-1.168.514-1.635.514-.748 0-.935-.234-1.495-.934l-4.577-7.186v6.952l1.449.327s0 .84-1.168.84l-3.222.187c-.093-.187 0-.653.327-.746l.84-.233V9.854L7.822 9.76c-.094-.42.14-1.026.793-1.073l3.456-.233 4.764 7.279v-6.44l-1.215-.14c-.093-.513.28-.886.747-.933l3.222-.187zM1.936 1.035l13.31-.98c1.634-.14 2.054-.047 3.082.7l4.249 2.986c.7.513.934.653.934 1.213v16.378c0 1.026-.373 1.634-1.68 1.726l-15.458.934c-.98.047-1.448-.093-1.962-.747l-3.129-4.06c-.56-.747-.793-1.306-.793-1.96V2.667c0-.839.374-1.54 1.447-1.632z"
        fillRule="evenodd"
      />
    </svg>
  ),
  jira: (
    <svg width="24" height="24" viewBox="0 0 24 24" fill="currentColor">
      <path d="M11.571 11.513H0a5.218 5.218 0 0 0 5.232 5.215h2.13v2.057A5.215 5.215 0 0 0 12.575 24V12.518a1.005 1.005 0 0 0-1.005-1.005z" />
      <path
        d="M5.024 5.247h11.571a5.218 5.218 0 0 0 5.232 5.215h-2.13v2.057A5.215 5.215 0 0 0 14.484 17.734V6.252a1.005 1.005 0 0 0-1.005-1.005H5.024z"
        opacity=".7"
        transform="translate(-1.732 -.247)"
      />
      <path
        d="M11.571 0H0a5.218 5.218 0 0 0 5.232 5.215h2.13v2.057A5.215 5.215 0 0 0 12.575 12.487V1.005A1.005 1.005 0 0 0 11.571 0z"
        opacity=".4"
        transform="translate(5.476)"
      />
    </svg>
  ),
  twilio: (
    <svg width="24" height="24" viewBox="0 0 24 24" fill="currentColor">
      <path d="M12 0C5.373 0 0 5.373 0 12s5.373 12 12 12 12-5.373 12-12S18.627 0 12 0zm0 20c-4.418 0-8-3.582-8-8s3.582-8 8-8 8 3.582 8 8-3.582 8-8 8zm-2-11a2 2 0 1 0 0-4 2 2 0 0 0 0 4zm4 0a2 2 0 1 0 0-4 2 2 0 0 0 0 4zm-4 4a2 2 0 1 0 0 4 2 2 0 0 0 0-4zm4 0a2 2 0 1 0 0 4 2 2 0 0 0 0-4z" />
    </svg>
  ),
  sendgrid: (
    <svg
      width="24"
      height="24"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <rect x="2" y="4" width="20" height="16" rx="2" />
      <polyline points="22,6 12,13 2,6" />
      <path d="M2 20l7-7" />
      <path d="M22 20l-7-7" />
    </svg>
  ),
  vercel: (
    <svg width="24" height="24" viewBox="0 0 24 24" fill="currentColor">
      <path d="M12 1L24 22H0L12 1z" />
    </svg>
  ),
  digitalocean: (
    <svg width="24" height="24" viewBox="0 0 24 24" fill="currentColor">
      <path d="M12.04 2C6.51 2 2 6.5 2 12.04c0 4.19 2.58 7.77 6.23 9.26v-4.72H5.68v-2.5h2.55v-2.28c0-1.53.48-3.36 3.11-3.36h2.7v2.43h-1.66c-.72 0-1.12.22-1.12.83v2.38h2.82l-.35 2.5h-2.47v4.67A10.04 10.04 0 0 0 22.04 12C22.04 6.5 17.54 2 12.04 2z" />
      <path d="M11.27 21.3v-3.22h-3.2v3.22h3.2z" opacity=".7" />
      <path d="M8.07 18.08v-2.67H5.73v2.67h2.34z" opacity=".5" />
    </svg>
  ),
  supabase: (
    <svg width="24" height="24" viewBox="0 0 24 24" fill="currentColor">
      <path
        d="M13.7 21.7c-.5.6-1.5.2-1.5-.6V13h8.6c1 0 1.5 1.1.8 1.8L13.7 21.7z"
        opacity=".6"
      />
      <path d="M10.3 2.3c.5-.6 1.5-.2 1.5.6V11H3.2c-1 0-1.5-1.1-.8-1.8L10.3 2.3z" />
    </svg>
  ),
  shopify: (
    <svg width="24" height="24" viewBox="0 0 24 24" fill="currentColor">
      <path d="M15.337 2.279s-.076.027-.203.074c-.124-.36-.343-.693-.65-.96 0 0-.473-.384-1.107-.384-.067 0-.138.006-.211.018-1.015-.636-2.02.056-2.646.55-.892.7-1.52 1.716-1.87 2.628-.973.304-1.647.512-1.735.54-.54.17-.557.187-.627.693-.054.38-1.465 11.293-1.465 11.293L14.464 19l6.553-1.424S15.415 2.3 15.337 2.279z" />
    </svg>
  ),
  hubspot: (
    <svg width="24" height="24" viewBox="0 0 24 24" fill="currentColor">
      <path d="M17.02 8.99V5.73a2.18 2.18 0 0 0 1.26-1.97c0-1.21-.98-2.19-2.19-2.19-1.21 0-2.19.98-2.19 2.19 0 .87.51 1.62 1.24 1.97v3.26a5.57 5.57 0 0 0-2.61 1.24L6.07 5.59A2.62 2.62 0 0 0 6.2 4.8c0-1.45-1.18-2.62-2.62-2.62S.96 3.35.96 4.8s1.18 2.62 2.62 2.62c.52 0 1.01-.16 1.42-.42l6.31 4.59a5.56 5.56 0 0 0-.1 7.85l-1.8 1.8a1.76 1.76 0 0 0-.54-.09 1.78 1.78 0 1 0 1.78 1.78c0-.19-.04-.37-.09-.54l1.8-1.8a5.58 5.58 0 1 0 4.66-11.6z" />
    </svg>
  ),
  openai: (
    <svg width="24" height="24" viewBox="0 0 24 24" fill="currentColor">
      <path d="M22.282 9.821a5.985 5.985 0 0 0-.516-4.91 6.046 6.046 0 0 0-6.51-2.9A6.065 6.065 0 0 0 4.981 4.18a5.998 5.998 0 0 0-3.998 2.9 6.046 6.046 0 0 0 .743 7.097 5.98 5.98 0 0 0 .51 4.911 6.051 6.051 0 0 0 6.516 2.9A5.985 5.985 0 0 0 13.26 24a6.056 6.056 0 0 0 5.772-4.206 5.99 5.99 0 0 0 3.997-2.9 6.056 6.056 0 0 0-.747-7.073zM13.26 22.43a4.476 4.476 0 0 1-2.876-1.04l.141-.081 4.779-2.758a.795.795 0 0 0 .392-.681v-6.737l2.02 1.168a.071.071 0 0 1 .038.052v5.583a4.504 4.504 0 0 1-4.494 4.494zM3.6 18.304a4.47 4.47 0 0 1-.535-3.014l.142.085 4.783 2.759a.771.771 0 0 0 .78 0l5.843-3.369v2.332a.08.08 0 0 1-.033.062L9.74 19.95a4.5 4.5 0 0 1-6.14-1.646zM2.34 7.896a4.485 4.485 0 0 1 2.366-1.973V11.6a.766.766 0 0 0 .388.676l5.815 3.355-2.02 1.168a.076.076 0 0 1-.071.005l-4.83-2.786A4.504 4.504 0 0 1 2.34 7.872v.024zm17.168 3.998l-5.816-3.356 2.02-1.164a.076.076 0 0 1 .071-.005l4.83 2.786a4.494 4.494 0 0 1-.694 8.086v-5.686a.785.785 0 0 0-.41-.66zm2.01-3.024l-.141-.085-4.774-2.782a.776.776 0 0 0-.785 0L9.975 9.372V7.04a.073.073 0 0 1 .032-.063l4.83-2.786a4.5 4.5 0 0 1 6.68 4.66v.018zM8.834 12.921l-2.02-1.164a.08.08 0 0 1-.038-.057V6.113a4.5 4.5 0 0 1 7.375-3.453l-.142.08L9.23 5.5a.795.795 0 0 0-.393.681l-.003 6.74zm1.098-2.367l2.602-1.5 2.602 1.5v3.001l-2.6 1.5-2.6-1.5V10.554z" />
    </svg>
  ),
  anthropic: (
    <svg width="24" height="24" viewBox="0 0 24 24" fill="currentColor">
      <path d="M13.827 3L21 21h-3.464l-1.57-3.927H10.59L12.061 13.5h5.404L13.827 3z" />
      <path
        d="M8.37 3L1 21h3.525l1.554-3.927h5.358L10.035 13.5H4.62L8.37 3z"
        opacity=".7"
      />
    </svg>
  ),
  sentry: (
    <svg width="24" height="24" viewBox="0 0 24 24" fill="currentColor">
      <path d="M13.91 2.505c-.873-1.553-3.066-1.553-3.94 0L7.092 7.293a8.012 8.012 0 0 1 4.596 6.37h-2.2a5.818 5.818 0 0 0-3.2-4.378L4.038 13.04a3.632 3.632 0 0 1 1.65 2.624H2.5a.75.75 0 0 1 0-1.5h1.2a1.5 1.5 0 0 0-.106-.466L1.51 18.54a.908.908 0 0 0 .78 1.37h4.92a.75.75 0 0 1 0 1.5H2.29a2.408 2.408 0 0 1-2.083-3.627l8.17-14.175a3.6 3.6 0 0 1 5.247-.005l8.17 14.175a2.408 2.408 0 0 1-2.083 3.627H16.8a.75.75 0 0 1 0-1.5h4.91a.908.908 0 0 0 .78-1.37L13.91 2.505z" />
    </svg>
  ),
  datadog: (
    <svg width="24" height="24" viewBox="0 0 24 24" fill="currentColor">
      <path d="M18.344 7.504l-1.078-.588-.582-.217-.07-.452.386-.37-.29-.678-1.07-.145-.767-.356-.147-.61.568-.673-.567-.563-.99.217-.617-.563-.39.06-.327.59-.628.1-.73-.343-.92.33-.344-.304-.44.317-.507-.317-.78.66.14.78-.386.42-.11.674.424.08.11.56-.11.5.423.237-.017.657-.36.457.098.217.377-.284.424.107.22-.41.534.053.527-.428.457.13.767-.13.364.27.457-.27.67-.106 1.3 1.12.327.95-.147.588.344.327.344-.414 1.076-.67-.29-.174-.648 1.05-1.04-.22-.89-.58-.11-.507.36-.527-.29-.42 1.006-1.06.11-.844-.44-.283-.73.33-.867-.47-.447.77-1.038z" />
    </svg>
  ),
};

const FALLBACK_LOGO = (
  <svg
    width="24"
    height="24"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="1.5"
    strokeLinecap="round"
    strokeLinejoin="round"
  >
    <path d="M18 8h1a4 4 0 0 1 0 8h-1" />
    <path d="M6 8H5a4 4 0 0 0 0 8h1" />
    <line x1="8" y1="12" x2="16" y2="12" />
  </svg>
);

const STATUS_COLORS = {
  connected: "var(--color-ok)",
  not_connected: "var(--color-dim)",
  error: "var(--color-err)",
};

// ── Main component ────────────────────────────────────────────────────────

export default function ConnectorsTab() {
  const [connectors, setConnectors] = useState(null);
  const [templates, setTemplates] = useState([]);
  const [vaultKeys, setVaultKeys] = useState(new Set());
  const [status, setStatus] = useState(null);
  const [saving, setSaving] = useState(null);
  const [selected, setSelected] = useState(null);
  const [secretValues, setSecretValues] = useState({});
  const [editConfig, setEditConfig] = useState({});
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [namingType, setNamingType] = useState(null);
  const [instanceName, setInstanceName] = useState("");
  const [search, setSearch] = useState("");
  const [oauthProxyUrl, setOauthProxyUrl] = useState(null);
  const [proxyProviders, setProxyProviders] = useState({});
  const [oauthPolling, setOauthPolling] = useState(null); // { sessionId, provider }

  const load = useCallback(() => {
    Promise.all([
      fetch("/api/settings/connectors", { headers: apiHeaders() }).then((r) =>
        r.json(),
      ),
      fetch("/api/settings/connector-templates", {
        headers: apiHeaders(),
      }).then((r) => r.json()),
      fetch("/api/settings/vault", { headers: apiHeaders() }).then((r) =>
        r.json(),
      ),
    ])
      .then(([c, t, v]) => {
        setConnectors(Array.isArray(c) ? c : c.connectors || []);
        setTemplates(Array.isArray(t) ? t : t.templates || []);
        const entries = v.entries || (Array.isArray(v) ? v : []);
        setVaultKeys(new Set(entries.map((e) => e.key)));
      })
      .catch(() =>
        setStatus({ type: "error", text: "Failed to load connectors" }),
      );
  }, []);

  useEffect(load, [load]);

  // Load OAuth proxy config and available providers
  useEffect(() => {
    fetch("/api/config", { headers: apiHeaders() })
      .then((r) => r.json())
      .then((cfg) => {
        if (cfg.oauth_proxy_url) {
          setOauthProxyUrl(cfg.oauth_proxy_url);
          fetch(`${cfg.oauth_proxy_url}/api/v1/oauth/providers`)
            .then((r) => r.json())
            .then((data) => {
              // Convert array [{name, scopes}] to map {name: {scopes}}
              const map = {};
              const arr = Array.isArray(data) ? data : data.providers || [];
              arr.forEach((p) => {
                map[p.name] = { scopes: p.scopes };
              });
              setProxyProviders(map);
            })
            .catch(() => {});
        }
      })
      .catch(() => {});
  }, []);

  useEffect(() => {
    const handler = (e) => {
      if (e.data?.type === "oauth-complete") {
        load();
        setStatus({ type: "ok", text: `Connected via OAuth` });
      }
    };
    window.addEventListener("message", handler);
    return () => window.removeEventListener("message", handler);
  }, [load]);

  const filteredTemplates = useMemo(() => {
    if (!search.trim()) return templates;
    const q = search.toLowerCase();
    return templates.filter(
      (t) =>
        t.name.toLowerCase().includes(q) ||
        t.display_name.toLowerCase().includes(q) ||
        (t.description || "").toLowerCase().includes(q),
    );
  }, [templates, search]);

  if (!connectors) return <Loading />;

  const handleAdd = async (templateName, name) => {
    setSaving("add");
    setStatus(null);
    try {
      const body = { type: templateName };
      if (name) body.name = name;
      const resp = await fetch("/api/settings/connectors", {
        method: "POST",
        headers: apiHeaders(),
        body: JSON.stringify(body),
      });
      if (resp.ok) {
        const created = await resp.json();
        setStatus({ type: "ok", text: `Added ${created.display_name}` });
        load();
        openDetail(created);
      } else {
        const err = await resp.json().catch(() => ({}));
        setStatus({ type: "error", text: err.error || "Failed to add" });
      }
    } catch (e) {
      setStatus({ type: "error", text: e.message });
    }
    setSaving(null);
  };

  const handleTileClick = (tpl) => {
    // For multi-instance, find all connectors of this type
    const conns = connectors.filter((c) => c.type === tpl.name);
    if (conns.length === 1) {
      openDetail(conns[0]);
    } else if (conns.length > 1) {
      // If multiple instances, open the first one (user can browse via grid for named ones)
      openDetail(conns[0]);
    } else if (tpl.multi_instance) {
      setNamingType(tpl.name);
      setInstanceName("");
    } else {
      handleAdd(tpl.name);
    }
  };

  const handleNameSubmit = () => {
    if (!instanceName.trim()) return;
    handleAdd(namingType, instanceName.trim());
    setNamingType(null);
    setInstanceName("");
  };

  const handleDelete = async (name) => {
    setSaving("del");
    setStatus(null);
    try {
      const resp = await fetch(
        `/api/settings/connectors/${encodeURIComponent(name)}`,
        {
          method: "DELETE",
          headers: apiHeaders(),
        },
      );
      if (resp.ok) {
        setSelected(null);
        setConfirmDelete(false);
        setStatus({ type: "ok", text: "Removed" });
        load();
      } else {
        setStatus({ type: "error", text: "Failed to delete" });
      }
    } catch (e) {
      setStatus({ type: "error", text: e.message });
    }
    setSaving(null);
  };

  const handleSaveSecret = async (key) => {
    const value = secretValues[key];
    if (!value?.trim()) return;
    setSaving(key);
    setStatus(null);
    try {
      const resp = await fetch(
        `/api/settings/vault/${encodeURIComponent(key)}`,
        {
          method: "PUT",
          headers: apiHeaders(),
          body: JSON.stringify({ value: value.trim() }),
        },
      );
      if (resp.ok) {
        setSecretValues((prev) => ({ ...prev, [key]: "" }));
        setStatus({ type: "ok", text: `Saved ${key}` });
        load();
      } else {
        setStatus({ type: "error", text: `Failed to save ${key}` });
      }
    } catch (e) {
      setStatus({ type: "error", text: e.message });
    }
    setSaving(null);
  };

  const handleSaveConfig = async (name) => {
    setSaving("config");
    setStatus(null);
    try {
      const resp = await fetch(
        `/api/settings/connectors/${encodeURIComponent(name)}`,
        {
          method: "PUT",
          headers: apiHeaders(),
          body: JSON.stringify({ config: editConfig }),
        },
      );
      if (resp.ok) {
        setStatus({ type: "ok", text: "Config saved" });
        load();
      } else {
        setStatus({ type: "error", text: "Failed to save config" });
      }
    } catch (e) {
      setStatus({ type: "error", text: e.message });
    }
    setSaving(null);
  };

  // OAuth via Spawner proxy
  const handleProxyOAuth = async (connName, providerKey) => {
    setSaving("oauth-connect");
    setStatus(null);
    const sessionId = crypto.randomUUID();
    const provider = proxyProviders[providerKey];
    const scopes = provider?.scopes?.join(",") || "";
    const url = `${oauthProxyUrl}/api/v1/oauth/connect/${encodeURIComponent(providerKey)}?session=${sessionId}&scopes=${encodeURIComponent(scopes)}`;

    const w = 600,
      h = 700;
    const left = window.screenX + (window.innerWidth - w) / 2;
    const top = window.screenY + (window.innerHeight - h) / 2;
    window.open(url, "oauth", `width=${w},height=${h},left=${left},top=${top}`);

    setOauthPolling({ sessionId, connName, providerKey });
    setStatus({ type: "ok", text: "Waiting for authorization..." });

    // Poll for completion
    const poll = async () => {
      const maxAttempts = 60; // 5 minutes at 5s intervals
      for (let i = 0; i < maxAttempts; i++) {
        await new Promise((r) => setTimeout(r, 3000));
        try {
          const resp = await fetch(
            `${oauthProxyUrl}/api/v1/oauth/sessions/${sessionId}`,
          );
          if (!resp.ok) continue;
          const data = await resp.json();
          if (data.status === "completed" && data.access_token) {
            // Store the token in the local vault
            const conn = connectors.find((c) => c.name === connName);
            const tokenKey =
              conn?.oauth_token_key ||
              connName.toUpperCase().replace(/-/g, "_") + "_TOKEN";
            await fetch(`/api/settings/vault/${encodeURIComponent(tokenKey)}`, {
              method: "PUT",
              headers: apiHeaders(),
              body: JSON.stringify({ value: data.access_token }),
            });
            // Store refresh token if present
            if (data.refresh_token) {
              const refreshKey =
                connName.toUpperCase().replace(/-/g, "_") + "_REFRESH_TOKEN";
              await fetch(
                `/api/settings/vault/${encodeURIComponent(refreshKey)}`,
                {
                  method: "PUT",
                  headers: apiHeaders(),
                  body: JSON.stringify({ value: data.refresh_token }),
                },
              );
            }
            // Update connector status + OAuth metadata
            const connUpdate = { status: "connected" };
            if (data.refresh_token) {
              connUpdate.oauth_refresh_key =
                connName.toUpperCase().replace(/-/g, "_") + "_REFRESH_TOKEN";
            }
            if (data.expires_in) {
              const expiresAt = new Date(
                Date.now() + data.expires_in * 1000,
              ).toISOString();
              connUpdate.oauth_expires_at = expiresAt;
            }
            await fetch(
              `/api/settings/connectors/${encodeURIComponent(connName)}`,
              {
                method: "PUT",
                headers: apiHeaders(),
                body: JSON.stringify(connUpdate),
              },
            );
            setStatus({ type: "ok", text: "Connected via OAuth" });
            setOauthPolling(null);
            setSaving(null);
            load();
            return;
          } else if (data.status === "error") {
            setStatus({
              type: "error",
              text: data.error_message || "OAuth failed",
            });
            setOauthPolling(null);
            setSaving(null);
            return;
          }
        } catch {
          // Network error, keep polling
        }
      }
      setStatus({ type: "error", text: "OAuth timed out" });
      setOauthPolling(null);
      setSaving(null);
    };
    poll();
  };

  const openDetail = (conn) => {
    setSelected(conn.name);
    setEditConfig(conn.config || {});
    setSecretValues({});
    setConfirmDelete(false);
    setStatus(null);
  };

  const selectedConn = connectors.find((c) => c.name === selected);
  const selectedTpl = selectedConn
    ? templates.find((t) => t.name === selectedConn.type)
    : null;

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "16px" }}>
      {/* Header + Search */}
      <div
        style={{
          display: "flex",
          alignItems: "flex-end",
          justifyContent: "space-between",
          gap: "16px",
        }}
      >
        <div>
          <div
            style={{
              fontSize: "13px",
              fontWeight: 600,
              color: "var(--color-secondary)",
              marginBottom: "4px",
            }}
          >
            Connectors
          </div>
          <div style={{ fontSize: "11px", color: "var(--color-dim)" }}>
            Connect external services. Click to configure.
          </div>
        </div>
        <div style={{ position: "relative", flexShrink: 0 }}>
          <svg
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="var(--color-dim)"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
            style={{
              position: "absolute",
              left: "8px",
              top: "50%",
              transform: "translateY(-50%)",
              pointerEvents: "none",
            }}
          >
            <circle cx="11" cy="11" r="8" />
            <line x1="21" y1="21" x2="16.65" y2="16.65" />
          </svg>
          <input
            className="s-input"
            placeholder="Filter..."
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            style={{
              width: "180px",
              fontSize: "12px",
              paddingLeft: "28px",
              fontFamily: "var(--font-mono)",
            }}
          />
        </div>
      </div>

      {/* Grid */}
      <div
        style={{
          display: "grid",
          gridTemplateColumns: "repeat(auto-fill, minmax(100px, 1fr))",
          gap: "1px",
          background: "var(--color-border-subtle)",
          border: "1px solid var(--color-border-subtle)",
        }}
      >
        {filteredTemplates.map((tpl) => {
          const conn = connectors.find((c) => c.type === tpl.name);
          const isConnected = conn?.status === "connected";
          const isConfigured = !!conn;
          const isActive = selected === conn?.name;

          return (
            <button
              key={tpl.name}
              onClick={() => handleTileClick(tpl)}
              disabled={saving === "add"}
              style={{
                display: "flex",
                flexDirection: "column",
                alignItems: "center",
                justifyContent: "center",
                gap: "8px",
                padding: "16px 8px",
                background: isActive
                  ? "var(--color-elevated)"
                  : isConfigured
                    ? "var(--color-surface)"
                    : "var(--color-bg)",
                border: "none",
                cursor: "pointer",
                transition: "background 0.15s ease",
                position: "relative",
              }}
              onMouseEnter={(e) => {
                if (!isActive)
                  e.currentTarget.style.background = "var(--color-elevated)";
              }}
              onMouseLeave={(e) => {
                if (!isActive)
                  e.currentTarget.style.background = isConfigured
                    ? "var(--color-surface)"
                    : "var(--color-bg)";
              }}
              title={
                isConfigured
                  ? `${tpl.display_name} — ${conn.status}`
                  : `Add ${tpl.display_name}`
              }
            >
              {isConfigured && (
                <span
                  style={{
                    position: "absolute",
                    top: 8,
                    right: 8,
                    width: 6,
                    height: 6,
                    borderRadius: "50%",
                    background: isConnected
                      ? "var(--color-ok)"
                      : "var(--color-dim)",
                  }}
                />
              )}
              <span
                style={{
                  color: isConfigured
                    ? "var(--color-primary)"
                    : "var(--color-dim)",
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "center",
                  transition: "color 0.15s ease",
                }}
              >
                {LOGOS[tpl.name] || FALLBACK_LOGO}
              </span>
              <span
                style={{
                  fontSize: "10px",
                  textAlign: "center",
                  lineHeight: 1.2,
                  maxWidth: "80px",
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                  whiteSpace: "nowrap",
                  color: isConfigured
                    ? "var(--color-secondary)"
                    : "var(--color-dim)",
                }}
              >
                {tpl.display_name}
              </span>
            </button>
          );
        })}
      </div>

      {filteredTemplates.length === 0 && (
        <div
          style={{
            textAlign: "center",
            padding: "24px",
            color: "var(--color-dim)",
            fontSize: "12px",
          }}
        >
          No connectors match "{search}"
        </div>
      )}

      {/* Multi-instance name prompt */}
      {namingType && (
        <div
          style={{
            border: "1px solid var(--color-border-subtle)",
            background: "var(--color-surface)",
            padding: "12px 16px",
            display: "flex",
            flexDirection: "column",
            gap: "8px",
          }}
        >
          <span style={{ fontSize: "12px", color: "var(--color-secondary)" }}>
            Name this{" "}
            {templates.find((t) => t.name === namingType)?.display_name}{" "}
            instance
          </span>
          <div style={{ display: "flex", gap: "8px" }}>
            <input
              className="s-input"
              placeholder="e.g. analytics-db"
              value={instanceName}
              onChange={(e) =>
                setInstanceName(
                  e.target.value.toLowerCase().replace(/[^a-z0-9-]/g, ""),
                )
              }
              onKeyDown={(e) => e.key === "Enter" && handleNameSubmit()}
              style={{
                flex: 1,
                fontFamily: "var(--font-mono)",
                fontSize: "13px",
              }}
              autoFocus
            />
            <button
              className="s-save-btn"
              style={{ padding: "4px 12px", fontSize: "12px" }}
              disabled={!instanceName.trim() || saving === "add"}
              onClick={handleNameSubmit}
            >
              {saving === "add" ? "..." : "Add"}
            </button>
            <button
              className="s-save-btn"
              style={{
                background: "transparent",
                color: "var(--color-muted)",
                border: "1px solid var(--color-border-main)",
                padding: "4px 12px",
                fontSize: "12px",
              }}
              onClick={() => setNamingType(null)}
            >
              Cancel
            </button>
          </div>
        </div>
      )}

      {/* Detail panel */}
      {selectedConn && (
        <div
          style={{
            border: "1px solid var(--color-border-subtle)",
            background: "var(--color-surface)",
          }}
        >
          {/* Header */}
          <div
            style={{
              display: "flex",
              alignItems: "center",
              justifyContent: "space-between",
              padding: "12px 16px",
              borderBottom: "1px solid var(--color-border-subtle)",
            }}
          >
            <div style={{ display: "flex", alignItems: "center", gap: "10px" }}>
              <span style={{ color: "var(--color-primary)", display: "flex" }}>
                {LOGOS[selectedConn.type] || FALLBACK_LOGO}
              </span>
              <div>
                <div
                  style={{
                    fontSize: "13px",
                    fontWeight: 600,
                    color: "var(--color-primary)",
                  }}
                >
                  {selectedConn.display_name}
                  {selectedConn.name !== selectedConn.type && (
                    <span
                      style={{
                        fontWeight: 400,
                        fontSize: "11px",
                        color: "var(--color-dim)",
                        marginLeft: "8px",
                        fontFamily: "var(--font-mono)",
                      }}
                    >
                      {selectedConn.name}
                    </span>
                  )}
                </div>
                <div style={{ fontSize: "11px", color: "var(--color-dim)" }}>
                  {selectedConn.description}
                </div>
              </div>
            </div>
            <div style={{ display: "flex", alignItems: "center", gap: "8px" }}>
              {selectedConn.status === "connected" && (
                <span
                  style={{
                    fontSize: "11px",
                    color: "var(--color-ok)",
                    display: "flex",
                    alignItems: "center",
                    gap: "4px",
                  }}
                >
                  <span
                    style={{
                      width: 6,
                      height: 6,
                      borderRadius: "50%",
                      background: "var(--color-ok)",
                    }}
                  />
                  Connected
                </span>
              )}
              <button
                style={{
                  background: "none",
                  border: "none",
                  color: "var(--color-dim)",
                  cursor: "pointer",
                  padding: "4px",
                  fontSize: "16px",
                  lineHeight: 1,
                }}
                onClick={() => setSelected(null)}
              >
                &times;
              </button>
            </div>
          </div>

          <div
            style={{
              padding: "12px 16px",
              display: "flex",
              flexDirection: "column",
              gap: "16px",
            }}
          >
            {/* Slack Socket Mode guided setup. Replaces the generic
                Secrets / OAuth UI for connectors flagged socket_mode in
                their template, because Socket Mode bots can't be set up
                via OAuth distribution and need a specific manifest+token
                walkthrough. */}
            {selectedTpl?.socket_mode && selectedConn.type === "slack" && (
              <SlackSetupGuide
                vaultKeys={vaultKeys}
                onSaved={(opts) => {
                  load();
                  if (opts && !opts.silent) {
                    setStatus({
                      type: "ok",
                      text: opts.text || "Connected",
                    });
                  }
                }}
                onError={(text) => setStatus({ type: "error", text })}
              />
            )}

            {/* Secrets */}
            {!selectedTpl?.socket_mode && selectedConn.secrets?.length > 0 && (
              <Section title="Secrets">
                {selectedConn.secrets.map((key) => {
                  const isSet = vaultKeys.has(key);
                  return (
                    <div
                      key={key}
                      style={{
                        display: "flex",
                        alignItems: "center",
                        gap: "8px",
                        minHeight: "32px",
                      }}
                    >
                      <span
                        style={{
                          fontSize: "12px",
                          fontFamily: "var(--font-mono)",
                          width: "200px",
                          flexShrink: 0,
                          color: isSet
                            ? "var(--color-secondary)"
                            : "var(--color-warn)",
                        }}
                      >
                        {key}
                      </span>
                      {isSet ? (
                        <span
                          style={{ fontSize: "11px", color: "var(--color-ok)" }}
                        >
                          &check; set
                        </span>
                      ) : (
                        <>
                          <input
                            className="s-input"
                            type="password"
                            placeholder="Paste value..."
                            value={secretValues[key] || ""}
                            onChange={(e) =>
                              setSecretValues((prev) => ({
                                ...prev,
                                [key]: e.target.value,
                              }))
                            }
                            style={{
                              flex: 1,
                              fontFamily: "var(--font-mono)",
                              fontSize: "12px",
                            }}
                          />
                          <button
                            className="s-save-btn"
                            style={{
                              padding: "4px 12px",
                              fontSize: "11px",
                              flexShrink: 0,
                            }}
                            disabled={
                              !secretValues[key]?.trim() || saving === key
                            }
                            onClick={() => handleSaveSecret(key)}
                          >
                            {saving === key ? "..." : "Save"}
                          </button>
                        </>
                      )}
                    </div>
                  );
                })}
              </Section>
            )}

            {/* OAuth (suppressed for socket_mode connectors — those use
                their own guided setup component above). */}
            {!selectedTpl?.socket_mode &&
              (selectedConn.auth_method === "oauth" ||
                selectedConn.oauth_token_url) &&
              selectedTpl?.has_oauth &&
              (() => {
                // Check if this connector's type matches a proxy provider
                const proxyKey = Object.keys(proxyProviders).find((k) => {
                  // Match provider key to connector type (e.g. "github-connector" → "github")
                  const base = k.replace(/-connector$/, "");
                  return selectedConn.type === base || selectedConn.type === k;
                });
                return (
                  <Section title="OAuth">
                    {selectedConn.oauth_token_key && (
                      <KVRow
                        label="Access token"
                        value={selectedConn.oauth_token_key}
                        ok={vaultKeys.has(selectedConn.oauth_token_key)}
                      />
                    )}
                    {selectedConn.oauth_refresh_key && (
                      <KVRow
                        label="Refresh token"
                        value={selectedConn.oauth_refresh_key}
                        ok={vaultKeys.has(selectedConn.oauth_refresh_key)}
                      />
                    )}
                    {selectedConn.oauth_expires_at && (
                      <KVRow
                        label="Expires"
                        value={selectedConn.oauth_expires_at}
                      />
                    )}
                    {(selectedTpl.oauth_scopes ||
                      proxyProviders[proxyKey]?.scopes) && (
                      <div
                        style={{
                          display: "flex",
                          alignItems: "center",
                          gap: "8px",
                          minHeight: "28px",
                        }}
                      >
                        <span
                          style={{
                            fontSize: "11px",
                            color: "var(--color-dim)",
                            width: "200px",
                            flexShrink: 0,
                          }}
                        >
                          Scopes
                        </span>
                        <div
                          style={{
                            display: "flex",
                            flexWrap: "wrap",
                            gap: "4px",
                          }}
                        >
                          {(
                            selectedTpl.oauth_scopes ||
                            proxyProviders[proxyKey]?.scopes ||
                            []
                          ).map((s) => (
                            <span
                              key={s}
                              style={{
                                padding: "1px 6px",
                                fontSize: "10px",
                                fontFamily: "var(--font-mono)",
                                border: "1px solid var(--color-border-main)",
                                color: "var(--color-dim)",
                              }}
                            >
                              {s}
                            </span>
                          ))}
                        </div>
                      </div>
                    )}

                    {oauthProxyUrl && proxyKey ? (
                      <div
                        style={{
                          marginTop: "8px",
                          display: "flex",
                          flexDirection: "column",
                          gap: "8px",
                        }}
                      >
                        <div
                          style={{
                            fontSize: "11px",
                            color: "var(--color-dim)",
                          }}
                        >
                          OAuth managed by Starpod — no credentials needed
                        </div>
                        {oauthPolling?.connName === selectedConn.name ? (
                          <div
                            style={{
                              display: "flex",
                              alignItems: "center",
                              gap: "8px",
                            }}
                          >
                            <span
                              style={{
                                display: "inline-block",
                                width: 8,
                                height: 8,
                                borderRadius: "50%",
                                background: "var(--color-accent)",
                                animation: "pulse 1.5s ease-in-out infinite",
                              }}
                            />
                            <span
                              style={{
                                fontSize: "12px",
                                color: "var(--color-secondary)",
                              }}
                            >
                              Waiting for authorization...
                            </span>
                          </div>
                        ) : (
                          <div
                            style={{
                              display: "flex",
                              justifyContent: "flex-end",
                            }}
                          >
                            <button
                              className="s-save-btn"
                              style={{ padding: "6px 16px", fontSize: "12px" }}
                              disabled={saving === "oauth-connect"}
                              onClick={() =>
                                handleProxyOAuth(selectedConn.name, proxyKey)
                              }
                            >
                              {selectedConn.status === "connected"
                                ? `Reconnect with ${selectedTpl.display_name}`
                                : `Connect with ${selectedTpl.display_name}`}
                            </button>
                          </div>
                        )}
                      </div>
                    ) : (
                      <div
                        style={{
                          marginTop: "8px",
                          fontSize: "11px",
                          color: "var(--color-dim)",
                        }}
                      >
                        OAuth not available — {oauthProxyUrl ? "provider not registered on proxy" : "no proxy configured"}
                      </div>
                    )}
                  </Section>
                );
              })()}

            {/* Config */}
            {Object.keys(editConfig).length > 0 && (
              <Section title="Configuration">
                {Object.entries(editConfig).map(([k, v]) => (
                  <div
                    key={k}
                    style={{
                      display: "flex",
                      alignItems: "center",
                      gap: "8px",
                      minHeight: "32px",
                    }}
                  >
                    <span
                      style={{
                        fontSize: "11px",
                        fontFamily: "var(--font-mono)",
                        color: "var(--color-secondary)",
                        width: "200px",
                        flexShrink: 0,
                      }}
                    >
                      {k}
                    </span>
                    <input
                      className="s-input"
                      value={v}
                      onChange={(e) =>
                        setEditConfig((prev) => ({
                          ...prev,
                          [k]: e.target.value,
                        }))
                      }
                      style={{
                        flex: 1,
                        fontFamily: "var(--font-mono)",
                        fontSize: "12px",
                      }}
                    />
                  </div>
                ))}
                <div
                  style={{
                    display: "flex",
                    justifyContent: "flex-end",
                    marginTop: "4px",
                  }}
                >
                  <button
                    className="s-save-btn"
                    style={{ padding: "4px 12px", fontSize: "11px" }}
                    disabled={saving === "config"}
                    onClick={() => handleSaveConfig(selectedConn.name)}
                  >
                    {saving === "config" ? "..." : "Save config"}
                  </button>
                </div>
              </Section>
            )}

            {/* Remove */}
            <div
              style={{
                borderTop: "1px solid var(--color-border-subtle)",
                paddingTop: "12px",
                display: "flex",
                justifyContent: "flex-end",
              }}
            >
              {confirmDelete ? (
                <div
                  style={{ display: "flex", alignItems: "center", gap: "8px" }}
                >
                  <span style={{ fontSize: "12px", color: "var(--color-err)" }}>
                    Remove this connector?
                  </span>
                  <button
                    className="s-save-btn"
                    style={{
                      background: "var(--color-err)",
                      padding: "4px 12px",
                      fontSize: "11px",
                    }}
                    onClick={() => handleDelete(selectedConn.name)}
                    disabled={saving === "del"}
                  >
                    {saving === "del" ? "..." : "Yes, remove"}
                  </button>
                  <button
                    className="s-save-btn"
                    style={{
                      background: "transparent",
                      color: "var(--color-muted)",
                      border: "1px solid var(--color-border-main)",
                      padding: "4px 12px",
                      fontSize: "11px",
                    }}
                    onClick={() => setConfirmDelete(false)}
                  >
                    Cancel
                  </button>
                </div>
              ) : (
                <button
                  style={{
                    background: "none",
                    border: "none",
                    color: "var(--color-dim)",
                    cursor: "pointer",
                    fontSize: "12px",
                    padding: "4px 0",
                  }}
                  onClick={() => setConfirmDelete(true)}
                >
                  Remove connector
                </button>
              )}
            </div>
          </div>
        </div>
      )}

      {/* Status toast */}
      {status && (
        <div
          style={{
            fontSize: "12px",
            color:
              status.type === "ok" ? "var(--color-ok)" : "var(--color-err)",
          }}
        >
          {status.text}
        </div>
      )}
    </div>
  );
}

// ── Helpers ────────────────────────────────────────────────────────────────

function Section({ title, children }) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "6px" }}>
      <span
        style={{
          fontSize: "10px",
          color: "var(--color-dim)",
          textTransform: "uppercase",
          letterSpacing: "0.08em",
          fontWeight: 600,
        }}
      >
        {title}
      </span>
      {children}
    </div>
  );
}

function KVRow({ label, value, ok }) {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: "8px",
        minHeight: "28px",
      }}
    >
      <span
        style={{
          fontSize: "11px",
          color: "var(--color-dim)",
          width: "200px",
          flexShrink: 0,
        }}
      >
        {label}
      </span>
      <span
        style={{
          fontSize: "11px",
          fontFamily: "var(--font-mono)",
          color:
            ok === true
              ? "var(--color-ok)"
              : ok === false
                ? "var(--color-warn)"
                : "var(--color-secondary)",
        }}
      >
        {value} {ok === true && "\u2713"}
        {ok === false && "(missing)"}
      </span>
    </div>
  );
}
