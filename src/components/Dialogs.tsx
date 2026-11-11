import { useStyle } from "../lib/style";
import { useEditor } from "../lib/store";
import { BugReportDialog } from "./BugReportDialog";
import { ExportDialog } from "./ExportDialog";
import { Launcher } from "./home/Launcher";
import { OpenedDraftDialog } from "./home/OpenedDraftDialog";
import { KeysQuestionDialog } from "./KeysQuestionDialog";
import { SwitchDialog } from "./home/SwitchConfirm";
import { ModelQuestionDialog } from "./ModelQuestionDialog";
import { SettingsDialog } from "./settings/SettingsDialog";
import { SetupDialog } from "./setup/SetupDialog";
import { ShareDialog } from "./ShareDialog";
import { LearnDialog } from "./style/LearnDialog";
import { YoutubeDialog } from "./YoutubeDialog";

export default function Dialogs() {
  const launcher = useEditor((s) => s.launcherOpen && s.view !== "home");
  // Learning from videos opens from Your style, the top bar or a toast, anywhere.
  const learnOpen = useStyle((s) => s.learnOpen);
  return (
    <>
      {launcher && <Launcher />}
      <ExportDialog />
      <YoutubeDialog />
      <SetupDialog />
      {learnOpen && <LearnDialog />}
      <SwitchDialog />
      <ModelQuestionDialog kind="speech" />
      <ModelQuestionDialog kind="vision" />
      <ModelQuestionDialog kind="scene" />
      <OpenedDraftDialog />
      <BugReportDialog />
      <ShareDialog />
      <KeysQuestionDialog />
      <SettingsDialog />
    </>
  );
}
