import { useStyle } from "../lib/style";
import { useEditor } from "../lib/store";
import { ChatGptDialog } from "./ChatGptDialog";
import { ConnectAgentDialog } from "./ConnectAgentDialog";
import { ExportDialog } from "./ExportDialog";
import { Launcher } from "./home/Launcher";
import { OpenedDraftDialog } from "./home/OpenedDraftDialog";
import { SwitchDialog } from "./home/SwitchConfirm";
import { ModelQuestionDialog } from "./ModelQuestionDialog";
import { SetupDialog } from "./setup/SetupDialog";
import { LearnDialog } from "./style/LearnDialog";
import { YoutubeDialog } from "./YoutubeDialog";

/** The launcher and the dialogs, each open only when asked for. */
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
      <ConnectAgentDialog />
      <ChatGptDialog />
      {learnOpen && <LearnDialog />}
      <SwitchDialog />
      <ModelQuestionDialog kind="speech" />
      <ModelQuestionDialog kind="vision" />
      <OpenedDraftDialog />
    </>
  );
}
