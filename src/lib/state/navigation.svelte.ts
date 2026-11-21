export const routes = [
  { id: "dashboard", label: "Dashboard" },
  { id: "phone-backup", label: "Phone Backup" },
  { id: "wifi-backup", label: "Wi-Fi Backup" },
  { id: "organizer", label: "Organizer" },
  { id: "duplicates", label: "Duplicates" },
  { id: "activity", label: "Activity" },
  { id: "settings", label: "Settings" },
] as const;

export type RouteId = (typeof routes)[number]["id"];

const DEFAULT_ROUTE: RouteId = "dashboard";

function isRouteId(value: string): value is RouteId {
  return routes.some((route) => route.id === value);
}

function readHash(): RouteId {
  const value = window.location.hash.replace(/^#\/?/, "");
  return isRouteId(value) ? value : DEFAULT_ROUTE;
}

class NavigationState {
  current = $state<RouteId>(readHash());

  constructor() {
    window.addEventListener("hashchange", this.handleHashChange);
    if (!window.location.hash) {
      window.history.replaceState(null, "", `#/${this.current}`);
    }
  }

  private handleHashChange = (): void => {
    this.current = readHash();
  };
}

export const navigation = new NavigationState();
