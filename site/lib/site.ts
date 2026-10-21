export const url = "https://nuzky.app";
export const description =
  "A free video editor for TikTok, Reels, Shorts and YouTube. AI writes the captions in your language and cuts the ums, pauses and retakes. No subscription, no watermark.";

export const repo = "https://github.com/nuzkyapp/nuzky";
export const releases = `${repo}/releases`;
export const buildFromSource = `${repo}/blob/main/docs/BUILDING.md`;
export const license = `${repo}/blob/main/LICENSE`;
export const bugReport = `${repo}/issues/new?template=bug_report.yml`;
const latest = `${releases}/latest`;

export type FileId = "dmg" | "exe" | "deb" | "appimage";

// The names the release workflow gives the installers (docs/BUILDING.md, Release files).
const fileName: Record<FileId, (version: string) => string> = {
  dmg: (v) => `Nuzky_${v}_aarch64.dmg`,
  exe: (v) => `Nuzky_${v}_x64-setup.exe`,
  deb: (v) => `Nuzky_${v}_amd64.deb`,
  appimage: (v) => `Nuzky_${v}_amd64.AppImage`,
};

/** The newest published release. Without its version every link leads to the latest release page. */
export type Release = {
  version: string | null;
  notes: string;
  files: Record<FileId, { href: string; name: string | null }>;
};

// A release version, x.y.z with an optional prerelease such as -alpha or -beta.2, and nothing else.
const semver = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(-(0|[1-9]\d*|\d*[A-Za-z-][\dA-Za-z-]*)(\.(0|[1-9]\d*|\d*[A-Za-z-][\dA-Za-z-]*))*)?$/;

// The version comes from the latest.json that installed copies read, never from package.json: main can be ahead
// of what is published. It goes into links, so only a valid version is used. Without the file (nothing published
// yet, no network) the page still renders, with every download leading to the latest release page.
export async function getRelease(): Promise<Release> {
  const version: string | null = await fetch(`${latest}/download/latest.json`, { next: { revalidate: 3600 }, signal: AbortSignal.timeout(5000) })
    .then((res) => (res.ok ? res.json() : null))
    .then((json) => (typeof json?.version === "string" && semver.test(json.version) ? json.version : null))
    .catch(() => null);
  const at = (file: string) => (version ? `${releases}/download/v${version}/${file}` : latest);
  const files = Object.fromEntries(
    (Object.keys(fileName) as FileId[]).map((id) => {
      const name = version ? fileName[id](version) : null;
      return [id, { href: name ? at(name) : latest, name }];
    }),
  ) as Release["files"];
  return { version, notes: version ? `${releases}/tag/v${version}` : latest, files };
}

export type SystemId = "mac" | "linux" | "windows";
export type Status = "tested" | "partly" | "untested";
export const statusWord: Record<Status, string> = { tested: "Tested", partly: "Partly tested", untested: "Not tested yet" };

// What each system needs and how far it is tested, from the most tested down. Claim nothing past this.
export const systems: {
  id: SystemId;
  name: string;
  requirement: string;
  status: Status;
  note: string;
  report?: string;
  warnings?: string[];
  files: { id: FileId; label: string; what: string; phone: string }[];
}[] = [
  {
    id: "mac",
    name: "Mac",
    requirement: "Apple silicon (M1 or newer), macOS 14 or newer",
    status: "tested",
    note: "I test it on my own Mac, and Apple notarized it.",
    files: [{ id: "dmg", label: "Download .dmg", what: "Disk image. Drag Nuzky to Applications.", phone: "A .dmg disk image" }],
  },
  {
    id: "linux",
    name: "Linux",
    requirement: "64-bit Intel or AMD PC (x86_64) with glibc 2.39 or newer",
    status: "partly",
    note: "Only partly tested so far, so expect more rough edges.",
    report: "Tell me what breaks",
    files: [
      { id: "deb", label: "Download .deb", what: "For Ubuntu 24.04 or newer", phone: "A .deb for Ubuntu 24.04 or newer" },
      { id: "appimage", label: "Download AppImage", what: "For other distributions", phone: "An AppImage for other distributions" },
    ],
  },
  {
    id: "windows",
    name: "Windows",
    requirement: "Windows 10 or 11 on a 64-bit Intel or AMD PC",
    status: "untested",
    note: "It builds, but nobody has tried it on a Windows PC yet.",
    report: "Tell me how it goes",
    warnings: [
      "Windows warns you the first time, because the installer isn't signed yet. Click More info, then Run anyway.",
      "AI editing is off on Windows, and captions take longer there.",
    ],
    files: [{ id: "exe", label: "Download installer", what: "Setup program (.exe)", phone: "A setup program (.exe)" }],
  },
];
