// Beta updates opt-in (#175), desktop only.
//
// The updater's endpoint is compiled into tauri.conf.json and points at
// /releases/latest/, which excludes pre-releases — that's the whole mechanism
// keeping a beta away from stable users (#171). The side effect is that a beta
// tester is cut off from the NEXT beta too, so each one is a manual install.
// With this on, the update check goes to the newest pre-release instead.
//
// localStorage like the other UI preferences; the backend has no opinion.

const KEY = "vellum-beta-updates";

let enabled = $state<boolean>(localStorage.getItem(KEY) === "1");

export const betaChannel = {
  get enabled() {
    return enabled;
  },
  set enabled(v: boolean) {
    enabled = v;
    localStorage.setItem(KEY, v ? "1" : "0");
  },
};
