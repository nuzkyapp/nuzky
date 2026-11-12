import { ComparisonPage, comparisonMetadata } from "@/components/ComparisonPage";

export const metadata = comparisonMetadata("/edits-alternative");

export default function Page() {
  return <ComparisonPage path="/edits-alternative" />;
}
