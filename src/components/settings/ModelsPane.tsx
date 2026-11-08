import { CaptionsStep } from "../setup/steps/CaptionsStep";
import { VisionStep } from "../setup/steps/VisionStep";
import { NamesSection } from "./NamesSection";
import { StepSection } from "./StepSection";

/** Settings › Models: the captions model and its language, the names it spells the user's way, and the face and person models, as in Get set up. */
export function ModelsPane() {
  return (
    <div className="flex flex-col gap-6">
      <StepSection title="Captions and transcript" id="speech">
        <CaptionsStep manage />
      </StepSection>
      <NamesSection />
      <StepSection title="Faces and people" id="vision">
        <VisionStep />
      </StepSection>
    </div>
  );
}
