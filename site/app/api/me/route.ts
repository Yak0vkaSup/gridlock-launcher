import { NextResponse } from "next/server";
import { requireLauncherUser } from "@/lib/launcher-user";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function GET(req: Request) {
  const user = await requireLauncherUser(req);
  if (user instanceof NextResponse) return user;
  return NextResponse.json(user);
}
