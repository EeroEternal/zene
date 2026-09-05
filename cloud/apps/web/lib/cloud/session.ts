import type {
  AuthResponse,
  LoginRequest,
  Organization,
  RegisterRequest,
  ResetPasswordRequest,
  SendVerificationCodeRequest,
  SendVerificationCodeResponse,
  User,
} from "../types.ts";
import { getJson, postJson } from "./http.ts";

export const meApi = {
  get: () => getJson<{ user: User; organization: Organization }>("/api/v1/me"),
};

export const authApi = {
  login: (req: LoginRequest) =>
    postJson<AuthResponse>("/api/v1/auth/login", req),
  sendCode: (req: SendVerificationCodeRequest) =>
    postJson<SendVerificationCodeResponse>("/api/v1/auth/send-code", req),
  register: (req: RegisterRequest) =>
    postJson<AuthResponse>("/api/v1/auth/register", req),
  resetPassword: (req: ResetPasswordRequest) =>
    postJson<AuthResponse>("/api/v1/auth/reset-password", req),
  email: (email: string) =>
    postJson<{ ok: boolean; loginUrl?: string }>("/api/v1/auth/email", { email }),
};
