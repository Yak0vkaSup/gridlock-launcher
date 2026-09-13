import { NextResponse } from "next/server";
import { requireLauncherUser } from "@/lib/launcher-user";
import { getJson, presign, type Manifest } from "@/lib/r2";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

const URL_SECONDS = 3600;

// The launcher asks for the newest build of its platform and gets the manifest with a presigned
// URL per file (content-addressed: objects/<sha256>), so only changed files are ever downloaded.
export async function GET(req: Request) {
  const user = await requireLauncherUser(req);
  if (user instanceof NextResponse) return user;

  const platform = new URL(req.url).searchParams.get("platform");
  if (platform !== "windows" && platform !== "linux") {
    return NextResponse.json({ error: "platform must be windows or linux" }, { status: 400 });
  }
  const latest = await getJson<{ version: string }>(`builds/${platform}/latest.json`);
  if (!latest) return NextResponse.json({ error: "no build published for this platform yet" }, { status: 404 });
  const manifest = await getJson<Manifest>(`builds/${platform}/${latest.version}.json`);
  if (!manifest) return NextResponse.json({ error: "manifest missing" }, { status: 500 });

  const files = await Promise.all(
    manifest.files.map(async (f) => ({ ...f, url: await presign(`objects/${f.sha256}`, URL_SECONDS) })),
  );
  return NextResponse.json({ ...manifest, files, urlExpiresAt: Date.now() + URL_SECONDS * 1000 });
}
