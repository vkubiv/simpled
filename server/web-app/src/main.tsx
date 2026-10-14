import "./index.css";

import { browser, MainClient } from "@simpled/backend-client";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { BrowserRouter } from "react-router-dom";
import { App } from "./app";
import { createAppDeps } from "./app-deps";
import { AccountSession, sessionFetch } from "./core/auth/account-session";

const root = document.getElementById("root");
if (!root) throw new Error("Root element not found");

/**
 * One stateless client: `/api` is the same origin (the gateway routes it to main-svc), so the browser carries the
 * HttpOnly session cookie, and any 401 tells the session it ended.
 */
const session = new AccountSession(browser);
const deps = createAppDeps({
  client: new MainClient({ baseUrl: "/api", fetch: sessionFetch(session) }),
  session,
});

createRoot(root).render(
  <StrictMode>
    <BrowserRouter>
      <App deps={deps} />
    </BrowserRouter>
  </StrictMode>,
);
