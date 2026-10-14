import { AuthApi } from "../api/auth";
import { InvitationsApi } from "../api/invitations";
import { MeApi } from "../api/me";
import { OrgsApi } from "../api/orgs";
import { ProjectsApi } from "../api/projects";
import { TeamsApi } from "../api/teams";
import { Transport, type TransportConfig } from "../transport";

/** main-svc: the web app's, the CLI's and the e2e suite's view of the control plane. */
export class MainClient {
  readonly auth: AuthApi;
  readonly me: MeApi;
  readonly orgs: OrgsApi;
  readonly invitations: InvitationsApi;
  readonly teams: TeamsApi;
  readonly projects: ProjectsApi;

  constructor(cfg: TransportConfig) {
    const t = new Transport(cfg);
    this.auth = new AuthApi(t);
    this.me = new MeApi(t);
    this.orgs = new OrgsApi(t);
    this.invitations = new InvitationsApi(t);
    this.teams = new TeamsApi(t);
    this.projects = new ProjectsApi(t);
  }
}
