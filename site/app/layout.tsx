import type { Metadata } from "next";
import { ClerkProvider, Show, SignInButton, UserButton } from "@clerk/nextjs";
import "./globals.css";

export const metadata: Metadata = {
  title: "GridLock",
  description: "GridLock closed demo: launcher and updates.",
  icons: { icon: "/favicon-32.png", apple: "/apple-touch-icon.png" },
};

const clerkAppearance = {
  variables: {
    colorPrimary: "#fd6f17",
    colorBackground: "#181818",
    colorText: "#f0e8e0",
    colorTextSecondary: "#a8a090",
    colorInputBackground: "#101010",
    colorInputText: "#f0e8e0",
    colorNeutral: "#f0e8e0",
    borderRadius: "6px",
  },
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <body>
        <ClerkProvider appearance={clerkAppearance}>
          <div className="shell">
            <header className="top">
              <Show when="signed-out">
                <SignInButton mode="modal">
                  <button className="signin">Sign in</button>
                </SignInButton>
              </Show>
              <Show when="signed-in">
                <UserButton />
              </Show>
            </header>
            {children}
          </div>
        </ClerkProvider>
      </body>
    </html>
  );
}
