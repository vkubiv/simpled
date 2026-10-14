import { env } from "./env";

interface MockMail {
  datetime: string;
  subject: string;
  content: { type: string; value: string }[];
}

/** The emails the backend sent through the mock mailer, newest first. */
export async function mailsTo(email: string): Promise<MockMail[]> {
  const res = await fetch(`${env.mailerUrl}/api/mails?to=${encodeURIComponent(email)}`);
  if (!res.ok) throw new Error(`Mock mailer answered ${res.status}`);
  const mails = (await res.json()) as MockMail[];
  return mails.sort((a, b) => b.datetime.localeCompare(a.datetime));
}

/**
 * The token of the newest link to the web app's `page` (`confirm-email`, `reset-password`, `invitation`) mailed to
 * `email`. Polls briefly: the backend sends after it saves, and the mock mailer stores asynchronously.
 */
export async function lastToken(email: string, page: string): Promise<string> {
  const marker = `${page}?token=`;
  for (let attempt = 0; attempt < 20; attempt++) {
    for (const mail of await mailsTo(email)) {
      const html = mail.content.map((c) => c.value).join("\n");
      const at = html.indexOf(marker);
      if (at >= 0) {
        const raw = html.slice(at + marker.length).match(/^[^'"&<\s]+/)?.[0] ?? "";
        return decodeURIComponent(raw);
      }
    }
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  throw new Error(`No ${page} link mailed to ${email}`);
}
