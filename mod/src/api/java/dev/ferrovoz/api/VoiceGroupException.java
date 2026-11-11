package dev.ferrovoz.api;

/** Something could not be done with a group. */
public final class VoiceGroupException extends RuntimeException {
    private static final long serialVersionUID = 1L;

    public enum Reason {
        INVALID_NAME,
        ALREADY_EXISTS,
        NOT_FOUND,
        WRONG_PASSWORD,
        /** The player doesn't have Ferrovoz or isn't online. */
        UNKNOWN_PLAYER,
        NOT_OWNER,
        NOT_IN_GROUP,
        /** Too many groups on the server, or too many owned by that player. */
        TOO_MANY,
        /** The voice relay isn't running. */
        NOT_RUNNING,
        UNKNOWN
    }

    private final Reason reason;

    public VoiceGroupException(Reason reason, String message) {
        super(message);
        this.reason = reason;
    }

    public Reason reason() {
        return reason;
    }

    /** Core error code (1-7) to its reason. */
    public static Reason reasonOf(int code) {
        return switch (code) {
            case 1 -> Reason.INVALID_NAME;
            case 2 -> Reason.ALREADY_EXISTS;
            case 3 -> Reason.NOT_FOUND;
            case 4 -> Reason.WRONG_PASSWORD;
            case 5 -> Reason.UNKNOWN_PLAYER;
            case 6 -> Reason.NOT_OWNER;
            case 7 -> Reason.NOT_IN_GROUP;
            case 8 -> Reason.TOO_MANY;
            default -> Reason.UNKNOWN;
        };
    }
}
