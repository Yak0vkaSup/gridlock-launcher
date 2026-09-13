import Image from "next/image";

type Asset = { name: string; browser_download_url: string; size: number };
type Release = { tag_name: string; html_url: string; assets: Asset[] };

const REPO = process.env.NEXT_PUBLIC_LAUNCHER_REPO ?? "Yak0vkaSup/gridlock-launcher";

async function latestLauncher(): Promise<Release | null> {
  try {
    // GITHUB_TOKEN (server-side, optional) lets this work while the launcher repo is private
    const headers: Record<string, string> = { accept: "application/vnd.github+json" };
    if (process.env.GITHUB_TOKEN) headers.authorization = `Bearer ${process.env.GITHUB_TOKEN}`;
    const res = await fetch(`https://api.github.com/repos/${REPO}/releases/latest`, { headers, next: { revalidate: 300 } });
    if (!res.ok) return null;
    return (await res.json()) as Release;
  } catch {
    return null;
  }
}

const mb = (n: number) => `${(n / 1048576).toFixed(0)} MB`;

export default async function Home() {
  const rel = await latestLauncher();
  const find = (pred: (n: string) => boolean) => rel?.assets.find((a) => pred(a.name.toLowerCase()));
  const win = find((n) => n.endsWith("-setup.exe"));
  const linux = find((n) => n.endsWith(".appimage"));

  return (
    <main className="hero">
      <Image src="/logo.png" alt="GridLock" width={2172} height={724} priority className="logo" />
      <p className="tagline">Closed demo. Install the launcher, sign in, play.</p>
      <div className="downloads">
        <a className={`btn primary ${win ? "" : "disabled"}`} href={win?.browser_download_url ?? "#"}>
          Windows <small>{win ? mb(win.size) : "soon"}</small>
        </a>
        <a className={`btn ${linux ? "" : "disabled"}`} href={linux?.browser_download_url ?? "#"}>
          Linux <small>{linux ? mb(linux.size) : "soon"}</small>
        </a>
      </div>
      <p className="hint">
        <b>Windows:</b> SmartScreen may warn about the unsigned launcher. More info, then Run anyway.
      </p>
      {rel ? <p className="version">launcher {rel.tag_name.replace("launcher-", "")}</p> : null}
      <a className="discord" href="https://discord.gg/Jqk43grKU" target="_blank" rel="noopener noreferrer">
        Discord
      </a>
    </main>
  );
}
