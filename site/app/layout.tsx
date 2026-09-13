import type { Metadata } from "next";
import { ClerkProvider, Show, SignInButton, UserButton } from "@clerk/nextjs";
import Link from "next/link";
import "./globals.css";

export const metadata: Metadata = {
  title: "GridLock",
  description: "GridLock demo access: launcher downloads and updates.",
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <body>
        <ClerkProvider>
          <div className="shell">
            <header className="top">
              <Link href="/" className="brand">GRID<span>LOCK</span></Link>
              <nav>
                <Show when="signed-out">
                  <SignInButton mode="modal">
                    <button className="signin">Sign in</button>
                  </SignInButton>
                </Show>
                <Show when="signed-in">
                  <UserButton />
                </Show>
              </nav>
            </header>
            {children}
          </div>
        </ClerkProvider>
      </body>
    </html>
  );
}
