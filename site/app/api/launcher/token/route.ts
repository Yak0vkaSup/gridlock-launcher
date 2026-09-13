import { auth } from "@clerk/nextjs/server";
import { NextResponse } from "next/server";
import { signLauncherToken } from "@/lib/token";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

// Called by /launcher/login after the Clerk sign-in; the browser session proves who is asking.
export async function POST() {
  const { userId } = await auth();
  if (!userId) return NextResponse.json({ error: "unauthorized" }, { status: 401 });
  return NextResponse.json({ token: await signLauncherToken(userId) });
}
