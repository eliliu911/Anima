import type { CapabilityCatalog } from "../../api/studio";

export const canvasCatalogFixture = {
  engines: ["zimage", "krea2", "krea2_edit", "qwen21_edit", "h3", "ltx"].map((id) => ({ id, label: id })),
  modes: [
    { id: "t2i", engines: ["zimage", "krea2"] },
    { id: "i2i", engines: ["zimage", "krea2"] },
    { id: "ref2i", engines: ["krea2_edit", "qwen21_edit"] },
    { id: "t2v", engines: ["h3", "ltx"] },
    { id: "i2v", engines: ["h3", "ltx"] },
    { id: "fl2v", engines: ["ltx", "h3"] },
    { id: "r2v", engines: ["h3"] },
  ], default_image_engine: "zimage", default_video_engine: "h3", aspects: [],
} as unknown as CapabilityCatalog;
