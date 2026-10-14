import { randomUUID } from "node:crypto";
import {
  type ApiContext,
  MainClient,
  type Me,
  MemoryCookieJar,
  type MyOrganization,
  type OrgRole,
} from "@simpled/backend-client";
import { env } from "./env";
import { expectSuccess } from "./expect";
import { lastToken } from "./mailbox";

export const PASSWORD = "correct-horse-battery";

interface Person {
  ctx: ApiContext;
  email: string;
  accountId: string;
}

/**
 * Scenario builder. People are named by alias ("ann"); their emails and organization names are unique per Sugar,
 * so tests share one long-lived backend and never clean up. Everything goes through the public client, the mock
 * mailer standing in for the inbox.
 */
export class Sugar {
  readonly client = new MainClient({ baseUrl: env.apiUrl, printLogs: env.printLogs });
  private readonly run = randomUUID().slice(0, 8);
  private readonly people = new Map<string, Person>();

  /** A unique address for `alias` in this Sugar; the same alias always gives the same address. */
  email(alias: string): string {
    return `${alias}-${this.run}@e2e.test`;
  }

  /** A unique organization name. */
  orgName(base: string): string {
    return `${base} ${this.run}`;
  }

  newContext(): ApiContext {
    return { cookieJar: new MemoryCookieJar() };
  }

  /** Registers, confirms through the mailed link (which signs in), and returns the person's first organization. */
  async register(alias: string, orgName = "Acme"): Promise<MyOrganization> {
    const ctx = this.newContext();
    const email = this.email(alias);
    expectSuccess(
      await this.client.auth.register(ctx, {
        email,
        password: PASSWORD,
        name: capitalize(alias),
        organizationName: this.orgName(orgName),
      }),
    );
    expectSuccess(await this.client.auth.confirmEmail(ctx, await lastToken(email, "confirm-email")));
    const me = await this.remember(alias, ctx);
    return me.organizations[0];
  }

  /** `inviter` invites `alias` with `role`; `alias` registers through the link and is signed in. */
  async invite(orgId: number, inviter: string, alias: string, role: OrgRole): Promise<void> {
    const email = this.email(alias);
    expectSuccess(await this.client.invitations.invite(this.ctx(inviter), orgId, email, role));
    const ctx = this.newContext();
    expectSuccess(
      await this.client.invitations.register(ctx, await lastToken(email, "invitation"), capitalize(alias), PASSWORD),
    );
    await this.remember(alias, ctx);
  }

  /** An organization owned by `owner` (registered first), with the other people invited by role. */
  async org(owner: string, members: Partial<Record<string, OrgRole>> = {}): Promise<number> {
    const { id } = await this.register(owner);
    for (const [alias, role] of Object.entries(members)) {
      if (role) await this.invite(id, owner, alias, role);
    }
    return id;
  }

  ctx(alias: string): ApiContext {
    return this.person(alias).ctx;
  }

  accountId(alias: string): string {
    return this.person(alias).accountId;
  }

  private person(alias: string): Person {
    const person = this.people.get(alias);
    if (!person) throw new Error(`No one called '${alias}' yet. Register or invite them first.`);
    return person;
  }

  private async remember(alias: string, ctx: ApiContext): Promise<Me> {
    const me = expectSuccess(await this.client.me.get(ctx));
    this.people.set(alias, { ctx, email: me.account.email, accountId: me.account.id });
    return me;
  }
}

function capitalize(alias: string): string {
  return alias.charAt(0).toUpperCase() + alias.slice(1);
}
