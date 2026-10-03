/** Minimum password length enforced by the API (`WEAK_PASSWORD` below it). */
export const PASSWORD_MIN_LENGTH = 12;

/** The API's password rules, shown under every field that sets a password. */
export const PASSWORD_HINT = `At least ${PASSWORD_MIN_LENGTH} characters, not a common password, not containing your email or username`;
