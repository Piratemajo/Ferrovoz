package dev.ferrovoz.api;

/**
 * Lo que fija un plugin sobre un permiso. Con {@link #INHERIT} manda el sistema de
 * permisos del servidor (LuckPerms, operadores...).
 */
public enum PermissionOverride {
    INHERIT,
    ALLOW,
    DENY;

    /** El valor final, dado el del sistema de permisos. */
    public boolean apply(boolean fromPermissions) {
        return switch (this) {
            case INHERIT -> fromPermissions;
            case ALLOW -> true;
            case DENY -> false;
        };
    }
}
