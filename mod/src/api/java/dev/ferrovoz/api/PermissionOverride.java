package dev.ferrovoz.api;

/**
 * What a plugin sets on a permission. With {@link #INHERIT}, the server's permission
 * system decides (LuckPerms, operators...).
 */
public enum PermissionOverride {
    INHERIT,
    ALLOW,
    DENY;

    /** The final value, given the one from the permission system. */
    public boolean apply(boolean fromPermissions) {
        return switch (this) {
            case INHERIT -> fromPermissions;
            case ALLOW -> true;
            case DENY -> false;
        };
    }
}
