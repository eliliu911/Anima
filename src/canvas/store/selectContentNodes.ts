import type { CanvasNode } from "../types";

/** Content-only subscription for DAG previews. Geometry must be read from getState()
 * at action time. One shared comparison per nodes array, rather than per subscriber.
 */
export function createContentNodesSelector() {
  let input: CanvasNode[] | undefined;
  let content: CanvasNode[] | undefined;
  return (state: { nodes: CanvasNode[] }): CanvasNode[] => {
    if (state.nodes === input) return content!;
    input = state.nodes;
    if (!content || content.length !== input.length || input.some((node, i) => {
      const previous = content![i];
      return node.id !== previous.id || node.type !== previous.type || node.data !== previous.data;
    })) content = input;
    return content;
  };
}

export const selectContentNodes = createContentNodesSelector();
