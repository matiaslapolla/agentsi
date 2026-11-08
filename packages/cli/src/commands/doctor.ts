import { doctor, doctorPassed, realProbes, type DoctorProbes, type DoctorResult } from "@agentsi/core";
import type { Command } from "../command.ts";

/** One line for each check: status, name with its level, and the version or what a failure blocks. */
export function doctorLines(results: DoctorResult[]): string[] {
  const names = results.map((r) => (r.level === "required" ? r.name : `${r.name} (${r.level})`));
  const width = Math.max(...names.map((n) => n.length));
  return results.map((r, i) => {
    const text = r.status === "ok" ? r.detail : `${r.status === "missing" ? "Not found." : `${r.detail}.`} It blocks ${r.blocks}.`;
    return `${r.status.padEnd(7)}  ${names[i]!.padEnd(width)}  ${text}`;
  });
}

export async function runDoctor(probes: DoctorProbes = realProbes): Promise<number> {
  const results = await doctor(probes);
  for (const line of doctorLines(results)) console.log(line);
  const passed = doctorPassed(results);
  console.log(passed ? "\nEvery required check passes." : "\nA required check failed. AgentSI cannot run until you fix it.");
  return passed ? 0 : 1;
}

export const doctorCommand: Command = {
  name: "doctor",
  summary: "Check the tools that AgentSI needs",
  usage: "agentsi doctor",
  run: () => runDoctor(),
};
