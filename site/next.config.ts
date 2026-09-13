import type { NextConfig } from "next";

// Old launchers (<= 0.2.0) open the browser on the *.vercel.app alias. Clerk production cookies live on
// gridlock.lat, so page routes hop there; /api/* stays reachable on every host (Bearer token, no cookies).
const OLD_HOSTS = ["gridlock-umber.vercel.app", "gridlock-yak0vkasups-projects.vercel.app"];

const nextConfig: NextConfig = {
  async redirects() {
    return OLD_HOSTS.map((host) => ({
      source: "/:path((?!api/).*)",
      has: [{ type: "host" as const, value: host }],
      destination: "https://gridlock.lat/:path",
      permanent: false,
    }));
  },
};

export default nextConfig;
