import { ref } from "vue";

/** The one image shown enlarged, if any. A single overlay serves every pane. */
export const lightbox = ref<{ src: string; alt: string }>();
export const openLightbox = (src: string, alt: string) => {
  lightbox.value = { src, alt };
};
export const closeLightbox = () => {
  lightbox.value = undefined;
};
