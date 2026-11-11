import type { ReactNode } from 'react';

export const metadata = {
  title: 'stele — slot NFTs with time-bounded tenancy',
  description:
    'A board of NEP-171 slot NFTs on NEAR. Positions are bought, rented and taken at a formula price, ' +
    'and a tenant writes content with a function-call key that expires with the term.',
};

/**
 * The demo's only chrome: a readable monospace column on a dark page.
 *
 * Styles are inline rather than a globals.css because the demo is one page with one layout; a
 * stylesheet here would be a third file whose only job is to restate this block.
 */
export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en">
      <body
        style={{
          margin: 0,
          background: '#0d1117',
          color: '#e6edf3',
          fontFamily: 'ui-monospace, SFMono-Regular, Menlo, Consolas, monospace',
          lineHeight: 1.55,
        }}
      >
        <main style={{ maxWidth: 880, margin: '0 auto', padding: '48px 24px 96px' }}>{children}</main>
      </body>
    </html>
  );
}
