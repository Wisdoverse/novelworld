/** @type {import('tailwindcss').Config} */
export default {
  content: ['./index.html', './src/**/*.{js,ts,jsx,tsx}'],
  theme: {
    extend: {
      colors: {
        void: '#03040a',
        cosmos: '#080d1f',
        nebula: '#0f1535',
        stardust: '#1a2040',
        aurora: {
          DEFAULT: '#6d28d9',
          light: '#8b5cf6',
        },
        nova: {
          DEFAULT: '#06b6d4',
          glow: '#22d3ee',
        },
        starlight: '#e2e8f0',
        moonbeam: '#94a3b8',
        comet: '#475569',
      },
      fontFamily: {
        display: ['Cinzel', 'serif'],
        body: ['Inter', 'sans-serif'],
        reading: ['Noto Serif SC', 'serif'],
      },
      animation: {
        'float': 'float 4s ease-in-out infinite',
        'pulse-glow': 'pulse-glow 2s ease-in-out infinite',
        'nebula': 'nebula-drift 20s ease-in-out infinite',
        'twinkle': 'twinkle 3s ease-in-out infinite',
      },
      backdropBlur: {
        xs: '2px',
      },
    },
  },
  plugins: [
    require('@tailwindcss/typography'),
  ],
};
