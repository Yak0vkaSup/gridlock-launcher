import { clerkClient } from "@clerk/nextjs/server";
import { NextResponse } from "next/server";
import { userIdFromRequest } from "./token";

export type LauncherUser = { id: string; email: string | null; name: string | null };

/** Resolves the launcher token to a live Clerk user, or the error response to return. */
export async function requireLauncherUser(req: Request): Promise<LauncherUser | NextResponse> {
  const userId = await userIdFromRequest(req);
  if (!userId) return NextResponse.json({ error: "unauthorized" }, { status: 401 });
  try {
    const user = await (await clerkClient()).users.getUser(userId);
    if (user.banned || user.locked) return NextResponse.json({ error: "forbidden" }, { status: 403 });
    return {
      id: user.id,
      email: user.primaryEmailAddress?.emailAddress ?? null,
      name: user.username ?? ([user.firstName, user.lastName].filter(Boolean).join(" ") || null),
    };
  } catch {
    return NextResponse.json({ error: "unauthorized" }, { status: 401 });
  }
}
