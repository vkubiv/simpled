import type { ApiContext } from "../context";
import type { ApiNotAuthorized } from "../errors";
import { parse } from "../model/common";
import { Me } from "../model/me";
import { forward, type Result } from "../result";
import type { Transport } from "../transport";
import { signedIn } from "./common";

export class MeApi {
  constructor(private readonly t: Transport) {}

  /** The signed-in account and its organizations; `ApiNotAuthorized` means signed out. */
  async get(ctx: ApiContext): Promise<Result<Me, ApiNotAuthorized>> {
    const r = await this.t.get(ctx, "/Me");
    return forward(r, (j) => parse(Me, j), signedIn);
  }
}
