import { SignUpButton, SignedOut } from "@clerk/nextjs";

type Asset = { name: string; browser_download_url: string; size: number };
type Release = { tag_name: string; html_url: string; assets: Asset[] };

const REPO = process.env.NEXT_PUBLIC_LAUNCHER_REPO ?? "Yak0vkaSup/gridlock-launcher";

async function latestLauncher(): Promise<Release | null> {
  try {
    const res = await fetch(`https://api.github.com/repos/${REPO}/releases/latest`, {
      headers: { accept: "application/vnd.github+json" },
      next: { revalidate: 300 },
    });
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
  const win = find((n) => n.endsWith("-setup.exe") || n.endsWith(".exe"));
  const appimage = find((n) => n.endsWith(".appimage"));
  const deb = find((n) => n.endsWith(".deb"));

  return (
    <main>
      <section className="hero">
        <h1>Closed demo</h1>
        <p>
          Install the launcher, sign in, press Play. The launcher keeps the game up to date:
          after every new build it downloads only what changed.
        </p>
        <div className="downloads">
          <a className={`btn primary ${win ? "" : "disabled"}`} href={win?.browser_download_url ?? "#"}>
            Windows <small>{win ? mb(win.size) : "soon"}</small>
          </a>
          <a className={`btn ${appimage ? "" : "disabled"}`} href={appimage?.browser_download_url ?? "#"}>
            Linux AppImage <small>{appimage ? mb(appimage.size) : "soon"}</small>
          </a>
          <a className={`btn ${deb ? "" : "disabled"}`} href={deb?.browser_download_url ?? "#"}>
            Linux .deb <small>{deb ? mb(deb.size) : "soon"}</small>
          </a>
        </div>
      </section>

      <section className="steps">
        <div className="step"><b>01 INSTALL</b>Get the launcher for your system. It is small; the game itself comes through it.</div>
        <div className="step"><b>02 SIGN IN</b>Create an account here, then press Sign in inside the launcher. It opens this site once and remembers you.</div>
        <div className="step"><b>03 PLAY</b>Install, then Play. Updates are picked up automatically on every start.</div>
      </section>

      <SignedOut>
        <p style={{ marginBottom: 24 }}>
          <SignUpButton mode="modal"><button className="btn">Create an account</button></SignUpButton>
        </p>
      </SignedOut>

      <p className="note">
        Windows may show a SmartScreen warning because the launcher is not code-signed yet:
        choose &ldquo;More info&rdquo;, then &ldquo;Run anyway&rdquo;.
        {rel ? <> Launcher {rel.tag_name}.</> : null}
      </p>
      <footer className="footer">GridLock demo. Builds are private and tied to your account.</footer>
    </main>
  );
}
