import { NextResponse, type NextRequest } from "next/server";
import { isLocalHost } from "@/lib/local-host";

export function proxy(request: NextRequest) {
  if (isLocalHost(request.headers.get("host"))) return NextResponse.next();
  return new NextResponse("Misdirected request: AgentSI answers only on 127.0.0.1, localhost or [::1].\n", { status: 421 });
}
