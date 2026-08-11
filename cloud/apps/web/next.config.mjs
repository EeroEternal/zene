/** @type {import('next').NextConfig} */
const isProd = process.env.NODE_ENV === "production";
const apiOrigin = (process.env.ZENE_CLOUD_API_URL || "http://127.0.0.1:8788").replace(
  /\/$/,
  "",
);

/** @type {import('next').NextConfig} */
const nextConfig = {
  // Static export is only for `npm run build` → dist/ (API hosts it).
  // Keep it off in `next dev` so rewrites/HMR work.
  ...(isProd ? { output: "export" } : {}),
  trailingSlash: false,
  images: { unoptimized: true },
};

if (!isProd) {
  nextConfig.rewrites = async () => [
    {
      source: "/api/:path*",
      destination: `${apiOrigin}/api/:path*`,
    },
  ];
}

export default nextConfig;
