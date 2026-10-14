# e2e guide

`server/e2e` drives the real stack: main-svc, Postgres and the mock mailer, through `@simpled/backend-client`,
and the web app's controllers through the same client. Nothing is mocked.

## Running

```sh
# from server/
just build               # the suite runs the Docker images: rebuild after backend or web-app changes
just test-e2e            # simpled test e2e --instance auto: a fresh stack of its own, torn down afterwards
just test-e2e-attached   # against what already runs on the gateway (`just run`, or `just infra` + IDE)

# from server/e2e, against a running stack
npx vitest run test/orgs.test.ts
npx vitest run -t "last owner"
PRINT_LOGS=1 npx vitest run test/auth.test.ts   # every request and response
```

`testspec.yaml` defines the suite: the `local` deployment in its own compose project (`test_env/`, empty
database), the suite's variables (`E2E_API_URL`, `E2E_MAILER_URL`, from the instance's named ports) and
`npm test` in `e2e/`. `--instance auto` takes the first instance whose ports are free, so a run works next to
your `just run` stack. `test/utils/env.ts` defaults to instance 0 (`:7080`, `:7025`) and also reads the `.env`
simpled wrote for the last run.

## Layout

```
e2e/test/
  utils/
    env.ts           URLs
    sugar.ts         Sugar: scenario builder
    mailbox.ts       mailsTo(email), lastToken(email, page): reads sendgrid-mock's GET /api/mails?to=
    expect.ts        expectSuccess / expectError
    web-app.ts       WebApp: the web app's deps over a real client; signedInApp(email)
    rx.ts            waitFor(rx, pred), settled(loadStatus)
  auth.test.ts  orgs.test.ts  invitations.test.ts  teams-projects.test.ts     the API
  web-app/onboarding.test.ts  web-app/organization.test.ts                    the web app's controllers
```

## Writing an API test

```ts
test("roles: admins manage non-owners, the last owner stays", async () => {
  const sugar = new Sugar();
  const { orgs } = sugar.client;
  const orgId = await sugar.org("ann", { bob: "admin", cid: "member" });

  expectSuccess(await orgs.setRole(sugar.ctx("bob"), orgId, sugar.accountId("cid"), "manager"));
  expectError(await orgs.setRole(sugar.ctx("bob"), orgId, sugar.accountId("ann"), "member"), RoleForbidden);
  expectError(await orgs.setRole(sugar.ctx("ann"), orgId, sugar.accountId("ann"), "admin"), LastOwner);
});
```

Rules:

- **One `Sugar` per test.** People are aliases (`"ann"`); `sugar.email(alias)` and `sugar.orgName(base)` are
  unique per Sugar, so tests share one long-lived database and never clean up.
- **Setup through the client.** `sugar.register(alias)` registers and confirms through the mailed link;
  `sugar.invite(orgId, inviter, alias, role)` invites and registers through the invitation;
  `sugar.org(owner, { alias: role })` does both. Each person gets a cookie-jar context: `sugar.ctx(alias)`,
  `sugar.accountId(alias)`. Grow `Sugar` when a setup repeats.
- **Tokens come from the mailbox.** `lastToken(email, "confirm-email" | "reset-password" | "invitation")`.
- **Assert on typed errors.** `expectError(r, Cls)` returns the error for its fields. A raw `ValidationError` in a
  test means the client is missing a mapping: fix the client.
- Plain-sentence test names, one file per area, no state shared between tests.

## Driving the web app

The web app's controllers need no DOM, so the suite runs them against the stack exactly as its unit tests do,
with real answers instead of fakes:

```ts
const owner = await signedInApp(sugar.email("ann"));    // a WebApp signed in through the sign-in op
await loadOrg({ ...owner.deps, orgId });                 // what the shell does before a page renders
const members = owner.own(new MembersController(owner.deps, { onLeft: vi.fn() }, orgId));
await settled(members.state.load, "members");
members.editInvite({ email: sugar.email("bob"), role: "manager" });
await members.sendInvite();
expect(members.state.inviteStatus.value.done).toBe(`Invitation sent to ${sugar.email("bob")}.`);
```

A `WebApp` is one browser: its own `AccountSession` and cookie jar. Dispose it after the test (the files collect
them and dispose in `afterEach`). Assert on controller state and routing spies, not on the API's shapes, which
the API tests cover. These tests found two session bugs the unit tests' fakes could not.

## Adding coverage

1. The endpoint in `backend-client` ([backend-client.md](backend-client.md)).
2. A test in the matching file, using `Sugar` (and `WebApp` for a page).
3. `just build && just test-e2e`, or `just test-e2e-attached` against a stack you are running.
