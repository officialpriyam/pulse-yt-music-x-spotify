import type { NextConfig } from "next";

const config: NextConfig = {
  async rewrites() {
    const backend = new URL(process.env.PULSE_BACKEND_URL || "http://127.0.0.1:4000");
    if (
      !["http:", "https:"].includes(backend.protocol) ||
      backend.pathname !== "/" || backend.search || backend.hash ||
      backend.username || backend.password
    ) {
      throw new Error("PULSE_BACKEND_URL must be the Rust backend origin.");
    }

    return [
      { source: "/auth/:path*", destination: `${backend.origin}/auth/:path*` },
      { source: "/api/v1/:path*", destination: `${backend.origin}/api/v1/:path*` },
    ];
  },
};

export default config;
