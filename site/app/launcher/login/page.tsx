"use client";

import { Suspense, useEffect, useState } from "react";
import { useSearchParams } from "next/navigation";

// The launcher opens this page with ?port=<loopback port>. Clerk (proxy.ts) makes sure the visitor is
// signed in; we mint the launcher token and hand it to the launcher's local listener.
function Handoff() {
  const params = useSearchParams();
  const port = Number(params.get("port"));
  const portOk = Number.isInteger(port) && port >= 1024 && port <= 65535;
  const [state, setState] = useState<"working" | "done" | "error">("working");
  const [detail, setDetail] = useState("");

  useEffect(() => {
    if (!portOk) return;
    (async () => {
      try {
        const res = await fetch("/api/launcher/token", { method: "POST" });
        if (!res.ok) throw new Error(`token: ${res.status}`);
        const { token } = (await res.json()) as { token: string };
        // a top-level navigation to the loopback address is allowed from https; the launcher answers
        // with its own "you can close this tab" page
        window.location.replace(`http://127.0.0.1:${port}/callback?token=${encodeURIComponent(token)}`);
        setState("done");
      } catch (e) {
        setState("error");
        setDetail(e instanceof Error ? e.message : String(e));
      }
    })();
  }, [port, portOk]);

  if (!portOk) {
    return (
      <div className="center">
        <div><h2>Open this page from the launcher</h2><p>Press Sign in inside the GridLock launcher.</p></div>
      </div>
    );
  }

  return (
    <div className="center">
      <div>
        {state === "working" && <><h2>Signing the launcher in…</h2><p>One moment.</p></>}
        {state === "done" && <><h2>Done</h2><p>You can close this tab and go back to the launcher.</p></>}
        {state === "error" && <><h2>Could not sign the launcher in</h2><p>{detail}</p></>}
      </div>
    </div>
  );
}

export default function LauncherLogin() {
  return (
    <Suspense fallback={<div className="center"><p>…</p></div>}>
      <Handoff />
    </Suspense>
  );
}
