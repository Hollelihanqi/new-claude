export interface UsageProfileSource {
  profile: string;
}

export interface UsageProfileOption {
  value: string;
  label: string;
}

export function buildUsageProfileOptions(
  rows: UsageProfileSource[],
  configuredProfiles: string[],
): UsageProfileOption[] {
  const profiles = Array.from(
    new Set([...configuredProfiles, ...rows.map((row) => row.profile)]),
  ).sort();

  return [
    { value: "__all__", label: "全部实例" },
    ...profiles.map((profile) => ({
      value: profile,
      label: profile === "__main__" ? "主账户" : profile,
    })),
  ];
}
