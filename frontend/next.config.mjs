// Production: a static export, served by the Rust server itself (single deployable).
// Development: `next dev` proxies /api to the Rust server so the SameSite session cookie stays same-origin.
const dev = process.env.NODE_ENV === 'development';

/** @type {import('next').NextConfig} */
const config = dev
  ? {
      async rewrites() {
        const target = process.env.API_ORIGIN ?? 'http://127.0.0.1:8080';
        return [{ source: '/api/:path*', destination: `${target}/api/:path*` }];
      },
    }
  : { output: 'export', trailingSlash: true, images: { unoptimized: true } };

export default config;
