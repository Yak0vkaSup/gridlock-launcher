import { clerkMiddleware, createRouteMatcher } from "@clerk/nextjs/server";

// Browser-facing pages that need a Clerk session. /api/* is checked by the launcher token instead.
const isProtected = createRouteMatcher(["/launcher(.*)"]);

export default clerkMiddleware(async (auth, req) => {
  if (isProtected(req)) await auth.protect();
});

export const config = {
  matcher: [
    "/((?!_next|[^?]*\\.(?:html?|css|js(?!on)|jpe?g|webp|png|gif|svg|ttf|woff2?|ico|csv|docx?|xlsx?|zip|webmanifest)).*)",
    "/(api|trpc)(.*)",
  ],
};
