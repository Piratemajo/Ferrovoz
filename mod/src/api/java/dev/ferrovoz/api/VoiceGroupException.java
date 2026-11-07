package dev.ferrovoz.api;

/** No se ha podido hacer algo con un grupo. */
public final class VoiceGroupException extends RuntimeException {
    public enum Reason {
        INVALID_NAME,
        ALREADY_EXISTS,
        NOT_FOUND,
        WRONG_PASSWORD,
        /** El jugador no tiene Ferrovoz o no está conectado. */
        UNKNOWN_PLAYER,
        NOT_OWNER,
        NOT_IN_GROUP,
        /** El relé de voz no está en marcha. */
        NOT_RUNNING,
        UNKNOWN
    }

    private static final long serialVersionUID = 1L;

    private final Reason reason;

    public VoiceGroupException(Reason reason, String message) {
        super(message);
        this.reason = reason;
    }

    public Reason reason() {
        return reason;
    }

    /** Código del núcleo (1-7) a su motivo. */
    public static Reason reasonOf(int code) {
        return switch (code) {
            case 1 -> Reason.INVALID_NAME;
            case 2 -> Reason.ALREADY_EXISTS;
            case 3 -> Reason.NOT_FOUND;
            case 4 -> Reason.WRONG_PASSWORD;
            case 5 -> Reason.UNKNOWN_PLAYER;
            case 6 -> Reason.NOT_OWNER;
            case 7 -> Reason.NOT_IN_GROUP;
            default -> Reason.UNKNOWN;
        };
    }
}
