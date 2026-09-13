import { SignJWT, jwtVerify } from "jose";

// The launcher holds a long-lived token of our own (30 days) instead of a Clerk session,
// signed with LAUNCHER_TOKEN_SECRET. /api/* checks it and then asks Clerk whether the user still exists.
const secret = () => new TextEncoder().encode(process.env.LAUNCHER_TOKEN_SECRET ?? "");
const ISSUER = "gridlock-site";

export async function signLauncherToken(userId: string): Promise<string> {
  return new SignJWT({})
    .setProtectedHeader({ alg: "HS256" })
    .setSubject(userId)
    .setIssuer(ISSUER)
    .setIssuedAt()
    .setExpirationTime("30d")
    .sign(secret());
}

export async function userIdFromRequest(req: Request): Promise<string | null> {
  const header = req.headers.get("authorization") ?? "";
  const token = header.startsWith("Bearer ") ? header.slice(7).trim() : "";
  if (!token) return null;
  try {
    const { payload } = await jwtVerify(token, secret(), { issuer: ISSUER });
    return payload.sub ?? null;
  } catch {
    return null;
  }
}
