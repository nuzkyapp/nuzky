import { headers } from "next/headers";
import { notFound } from "next/navigation";
import type { ReactNode } from "react";
import type { SearchParams } from "@/lib/oauth-query";
import { getServer } from "@/lib/server";
import { daysAgo, getTelemetry } from "@/lib/telemetry";

type Row = Record<string, string | number | null>;

function Panel({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="flex min-w-0 flex-col gap-3 rounded-xl bg-white/5.5 p-4 shadow-[inset_0_0_0_1px_rgb(255_255_255/.06)]">
      <h2 className="text-[13px] font-semibold">{title}</h2>
      {children}
    </section>
  );
}

function Table({ head, children }: { head: string[]; children: ReactNode }) {
  return (
    <div className="overflow-x-auto">
      <table className="w-full text-left text-[13px] tabular-nums">
        <thead>
          <tr>
            {head.map((name) => (
              <th key={name} className="pr-3 pb-1.5 text-xs font-medium whitespace-nowrap text-muted">
                {name}
              </th>
            ))}
          </tr>
        </thead>
        <tbody className="[&_td]:border-t [&_td]:border-white/6 [&_td]:py-1.5 [&_td]:pr-3 [&_td]:align-top">{children}</tbody>
      </table>
    </div>
  );
}

function Empty() {
  return <p className="text-xs text-muted">Nothing in this period.</p>;
}

function Expand({ summary, children }: { summary: ReactNode; children: ReactNode }) {
  return (
    <details className="group min-w-0">
      <summary className="cursor-pointer wrap-anywhere marker:text-subtle">{summary}</summary>
      <div className="mt-2 flex flex-col gap-2">{children}</div>
    </details>
  );
}

function Block({ children }: { children: ReactNode }) {
  return (
    <pre className="max-h-96 overflow-auto rounded-lg bg-black/30 p-2.5 text-xs leading-[18px] wrap-anywhere whitespace-pre-wrap">
      {children}
    </pre>
  );
}

function text(value: unknown): string {
  return typeof value === "string" ? value : value === undefined ? "" : JSON.stringify(value, null, 2);
}

function pretty(data: unknown): string {
  return JSON.stringify(JSON.parse(String(data)), null, 2);
}

function time(at: unknown): string {
  return new Date(Number(at)).toISOString().slice(0, 16).replace("T", " ");
}

function count(value: unknown): string {
  return Number(value).toLocaleString("en-US");
}

function median(values: number[]): number | null {
  if (!values.length) return null;
  const sorted = values.toSorted((a, b) => a - b);
  const middle = sorted.length >> 1;
  return sorted.length % 2 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2;
}

function Field({ label, value }: { label: string; value: unknown }) {
  return (
    <div className="flex flex-col gap-1">
      <span className="text-xs font-medium text-muted">{label}</span>
      <Block>{text(value)}</Block>
    </div>
  );
}

export default async function AdminPage({ searchParams }: { searchParams: SearchParams }) {
  const requestHeaders = await headers();
  const { auth, config } = await getServer();
  const session = await auth.api.getSession({ headers: requestHeaders });
  if (!session?.user.emailVerified || !config.adminEmails.includes(session.user.email.toLowerCase())) notFound();

  const days = (await searchParams).days === "30" ? 30 : 7;
  const since = daysAgo(days);
  const { database } = await getTelemetry();
  const all = (sql: string, ...params: (string | number)[]) => database.prepare(sql).all(...params) as Row[];

  const { installs } = database.prepare("SELECT count(DISTINCT install) AS installs FROM events WHERE received >= ?").get(since) as Row;
  const kinds = all(
    "SELECT kind, count(*) AS events, count(DISTINCT install) AS installs FROM events WHERE received >= ? GROUP BY kind ORDER BY events DESC",
    since,
  );
  const versions = all(
    "SELECT version AS name, count(DISTINCT install) AS installs FROM events WHERE received >= ? GROUP BY version ORDER BY installs DESC LIMIT 20",
    since,
  );
  const systems = all(
    "SELECT system AS name, count(DISTINCT install) AS installs FROM events WHERE received >= ? GROUP BY system ORDER BY installs DESC LIMIT 20",
    since,
  );
  const problems = all(
    `SELECT kind, CAST(CASE kind WHEN 'panic' THEN data ->> '$.location' ELSE data ->> '$.message' END AS TEXT) AS place,
       count(*) AS events, count(DISTINCT install) AS installs, max(at) AS newest, data
     FROM events WHERE kind IN ('panic', 'error') AND at >= ? GROUP BY kind, place ORDER BY events DESC LIMIT 50`,
    since,
  );
  const jobs = all(
    `SELECT CAST(e.data ->> '$.kind' AS TEXT) AS job, CAST(e.data ->> '$.error' AS TEXT) AS error,
       count(DISTINCT e.id) AS events, count(DISTINCT e.install) AS installs, json_group_array(DISTINCT CAST(f.value AS TEXT)) AS files
     FROM events e LEFT JOIN json_each(e.data, '$.files') f
     WHERE e.kind = 'job' AND e.at >= ? AND e.data ->> '$.status' = 'failed' GROUP BY job, error ORDER BY events DESC LIMIT 50`,
    since,
  );
  const turns = all(
    `SELECT CAST(data ->> '$.agent' AS TEXT) AS agent, CAST(data ->> '$.model' AS TEXT) AS model, count(*) AS turns,
       count(CASE WHEN data ->> '$.end' NOT IN ('done', 'stopped') THEN 1 END) AS errors,
       json_group_array(CAST(data ->> '$.seconds' AS REAL)) AS seconds
     FROM events WHERE kind = 'aiTurn' AND at >= ? GROUP BY agent, model ORDER BY turns DESC`,
    since,
  );
  const failedTools = all(
    `SELECT CAST(e.data ->> '$.agent' AS TEXT) AS agent, CAST(e.data ->> '$.model' AS TEXT) AS model, CAST(f.key AS TEXT) AS tool,
       total(f.value) AS failures
     FROM events e, json_each(e.data, '$.failed') f
     WHERE e.kind = 'aiTurn' AND e.at >= ? GROUP BY agent, model, tool ORDER BY failures DESC`,
    since,
  );
  const newest = all(
    "SELECT kind, at, install, version, system, data FROM events WHERE kind IN ('aiChat', 'report') AND at >= ? ORDER BY at DESC LIMIT 50",
    since,
  );

  const tabs = [7, 30].map((period) => (
    <a
      key={period}
      href={`/admin?days=${period}`}
      aria-current={period === days ? "page" : undefined}
      className="inline-flex h-7 items-center rounded-md px-3 text-xs font-medium text-muted aria-[current=page]:bg-white/12 aria-[current=page]:text-fg focus-visible:outline-2 focus-visible:outline-accent"
    >
      {period} days
    </a>
  ));

  return (
    <main className="flex min-h-dvh justify-center px-5 pt-10 pb-16">
      <div className="flex w-full max-w-[1040px] flex-col gap-3">
        <header className="mb-1 flex items-center justify-between gap-3">
          <h1 className="text-xl font-semibold">Telemetry</h1>
          <nav className="flex rounded-lg bg-white/5.5 p-0.5 shadow-[inset_0_0_0_1px_rgb(255_255_255/.06)]">{tabs}</nav>
        </header>

        <Panel title={`${count(installs)} installs sent something in the last ${days} days`}>
          <div className="grid gap-4 md:grid-cols-3">
            <Table head={["Kind", "Events", "Installs"]}>
              {kinds.map((row) => (
                <tr key={row.kind}>
                  <td className="wrap-anywhere">{row.kind}</td>
                  <td>{count(row.events)}</td>
                  <td>{count(row.installs)}</td>
                </tr>
              ))}
            </Table>
            <Table head={["Version", "Installs"]}>
              {versions.map((row) => (
                <tr key={row.name}>
                  <td className="wrap-anywhere">{row.name}</td>
                  <td>{count(row.installs)}</td>
                </tr>
              ))}
            </Table>
            <Table head={["System", "Installs"]}>
              {systems.map((row) => (
                <tr key={row.name}>
                  <td className="wrap-anywhere">{row.name}</td>
                  <td>{count(row.installs)}</td>
                </tr>
              ))}
            </Table>
          </div>
        </Panel>

        <Panel title="Crashes and errors">
          {problems.length ? (
            <Table head={["Where", "Kind", "Events", "Installs", "Newest"]}>
              {problems.map((row) => (
                <tr key={`${row.kind} ${row.place}`}>
                  <td className="w-full">
                    <Expand summary={row.place ?? "(none)"}>
                      <Block>{pretty(row.data)}</Block>
                    </Expand>
                  </td>
                  <td>{row.kind}</td>
                  <td>{count(row.events)}</td>
                  <td>{count(row.installs)}</td>
                  <td className="whitespace-nowrap">{time(row.newest)}</td>
                </tr>
              ))}
            </Table>
          ) : (
            <Empty />
          )}
        </Panel>

        <Panel title="Failed jobs">
          {jobs.length ? (
            <Table head={["Job", "Error", "Events", "Installs"]}>
              {jobs.map((row) => (
                <tr key={`${row.job} ${row.error}`}>
                  <td className="whitespace-nowrap">{row.job ?? "(none)"}</td>
                  <td className="w-full">
                    <Expand summary={row.error ?? "(none)"}>
                      <Field
                        label="Files"
                        value={(JSON.parse(String(row.files)) as (string | null)[]).filter((file) => file !== null).join("\n")}
                      />
                    </Expand>
                  </td>
                  <td>{count(row.events)}</td>
                  <td>{count(row.installs)}</td>
                </tr>
              ))}
            </Table>
          ) : (
            <Empty />
          )}
        </Panel>

        <Panel title="AI panel turns">
          {turns.length ? (
            <Table head={["Agent", "Model", "Turns", "Errors", "Median time", "Most failed tools"]}>
              {turns.map((row) => {
                const seconds = median((JSON.parse(String(row.seconds)) as (number | null)[]).filter((value) => value !== null));
                const tools = failedTools
                  .filter((tool) => tool.agent === row.agent && tool.model === row.model)
                  .slice(0, 5)
                  .map((tool) => `${tool.tool} ×${count(tool.failures)}`)
                  .join(", ");
                return (
                  <tr key={`${row.agent} ${row.model}`}>
                    <td className="whitespace-nowrap">{row.agent ?? "(none)"}</td>
                    <td className="whitespace-nowrap">{row.model ?? "(none)"}</td>
                    <td>{count(row.turns)}</td>
                    <td>{count(row.errors)}</td>
                    <td className="whitespace-nowrap">{seconds === null ? "" : `${seconds.toFixed(1)} s`}</td>
                    <td className="w-full wrap-anywhere">{tools}</td>
                  </tr>
                );
              })}
            </Table>
          ) : (
            <Empty />
          )}
        </Panel>

        <Panel title="Newest AI conversations and reports">
          {newest.length ? (
            <div className="flex flex-col divide-y divide-white/6">
              {newest.map((row, index) => {
                const data = JSON.parse(String(row.data)) as Record<string, unknown>;
                const chat = row.kind === "aiChat";
                return (
                  <div key={index} className="py-1.5">
                    <Expand
                      summary={
                        <>
                          <span className="text-muted">
                            {time(row.at)} · {chat ? `${text(data.agent)} ${text(data.model)}` : "Report"} ·{" "}
                          </span>
                          {text(chat ? data.prompt : data.text).slice(0, 160)}
                        </>
                      }
                    >
                      {chat ? (
                        <>
                          <Field label="Prompt" value={data.prompt} />
                          <Field label="Answer" value={data.answer} />
                          <Field label="Tools" value={data.tools} />
                          <Field label="End" value={data.end} />
                        </>
                      ) : (
                        <>
                          <Field label="Text" value={data.text} />
                          <Field label="Description" value={data.description} />
                        </>
                      )}
                      <p className="text-xs text-muted">
                        Install {String(row.install).slice(0, 12)} · {row.version} · {row.system}
                      </p>
                    </Expand>
                  </div>
                );
              })}
            </div>
          ) : (
            <Empty />
          )}
        </Panel>
      </div>
    </main>
  );
}
