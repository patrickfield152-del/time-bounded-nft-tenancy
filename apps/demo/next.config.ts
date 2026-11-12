import type { NextConfig } from 'next';

/**
 * The SDK and React packages export TypeScript sources in this workspace, so
 * Next has to transpile them rather than treat them as prebuilt packages.
 */
const nextConfig: NextConfig = {
  transpilePackages: ['@stele/sdk', '@stele/react'],
  reactStrictMode: true,
};

export default nextConfig;
