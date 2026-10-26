package io.github.thkprado.jlens;

/**
 * The result of an audited evaluation ({@link Expression#evaluateWithAudit(Object)}): its result
 * or its error, its audit and the explanation of it. An evaluation that fails still has its
 * audit, up to the failure.
 *
 * @param result the result as Java objects, {@code null} when it is undefined or {@code null},
 *     or when the evaluation failed
 * @param error the error, when the evaluation failed, or {@code null}. When a host function
 *     threw, its code is the exception's ({@link JlensException#code()}, or the simple name of
 *     its class) and its {@linkplain Throwable#getCause() cause} is the exception
 * @param audit the audit tree
 * @param explanation why the result was reached
 */
public record AuditResult(Object result, JlensException error, AuditTree audit,
        Explanation explanation) {}
