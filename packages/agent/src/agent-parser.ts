import * as fs from 'node:fs';
import * as ts from 'typescript';

export interface ParsedAgentFile {
	triggers: {
		webhook?: boolean;
	};
}

/** Extract static agent metadata at build time without evaluating the agent module. */
export function parseAgentFile(filePath: string): ParsedAgentFile {
	return {
		triggers: parseTriggers(filePath),
	};
}

function parseTriggers(filePath: string): ParsedAgentFile['triggers'] {
	const source = fs.readFileSync(filePath, 'utf-8');
	const ast = ts.createSourceFile(
		filePath,
		source,
		ts.ScriptTarget.Latest,
		true,
		scriptKindForFile(filePath),
	);
	let result: ParsedAgentFile['triggers'] | undefined;

	for (const statement of ast.statements) {
		if (isTriggersReExport(statement)) {
			throwUnsupportedTriggers(filePath, 're-exported triggers are not supported');
		}
		if (!ts.isVariableStatement(statement) || !hasExportModifier(statement)) continue;

		for (const declaration of statement.declarationList.declarations) {
			if (!ts.isIdentifier(declaration.name) || declaration.name.text !== 'triggers') continue;
			if (result) throwUnsupportedTriggers(filePath, 'multiple triggers exports were found');
			if (!declaration.initializer) throwUnsupportedTriggers(filePath, 'missing initializer');
			result = parseTriggersInitializer(filePath, declaration.initializer);
		}
	}

	return result ?? {};
}

function scriptKindForFile(filePath: string): ts.ScriptKind {
	if (/\.m?js$/.test(filePath)) return ts.ScriptKind.JS;
	return ts.ScriptKind.TS;
}

function hasExportModifier(statement: ts.VariableStatement): boolean {
	return statement.modifiers?.some((modifier) => modifier.kind === ts.SyntaxKind.ExportKeyword) ?? false;
}

function isTriggersReExport(statement: ts.Statement): boolean {
	if (!ts.isExportDeclaration(statement) || !statement.exportClause) return false;
	if (!ts.isNamedExports(statement.exportClause)) return false;
	return statement.exportClause.elements.some((element) => element.name.text === 'triggers');
}

function parseTriggersInitializer(
	filePath: string,
	initializer: ts.Expression,
): ParsedAgentFile['triggers'] {
	const expr = unwrapExpression(initializer);
	if (!ts.isObjectLiteralExpression(expr)) {
		throwUnsupportedTriggers(filePath, 'expected a static object literal');
	}

	const result: ParsedAgentFile['triggers'] = {};
	for (const property of expr.properties) {
		if (ts.isSpreadAssignment(property)) {
			throwUnsupportedTriggers(filePath, 'spread properties are not supported');
		}
		if (ts.isShorthandPropertyAssignment(property)) {
			const name = property.name.text;
			if (name === 'webhook') {
				throwUnsupportedTriggers(filePath, `"${name}" must use an explicit static value`);
			}
			continue;
		}
		if (!ts.isPropertyAssignment(property)) {
			const name = propertyNameText(filePath, property.name);
			if (name === 'webhook') {
				throwUnsupportedTriggers(filePath, `"${name}" must use an explicit static value`);
			}
			continue;
		}

		const name = propertyNameText(filePath, property.name);
		if (name === 'webhook') {
			const value = unwrapExpression(property.initializer);
			if (value.kind === ts.SyntaxKind.TrueKeyword) result.webhook = true;
			else if (value.kind === ts.SyntaxKind.FalseKeyword) delete result.webhook;
			else throwUnsupportedTriggers(filePath, '"webhook" must be true or false');
		}
	}

	return result;
}

function unwrapExpression(expr: ts.Expression): ts.Expression {
	while (
		ts.isAsExpression(expr) ||
		ts.isSatisfiesExpression(expr) ||
		ts.isTypeAssertionExpression(expr) ||
		ts.isParenthesizedExpression(expr)
	) {
		expr = expr.expression;
	}
	return expr;
}

function propertyNameText(filePath: string, name: ts.PropertyName): string | undefined {
	if (ts.isIdentifier(name) || ts.isStringLiteral(name) || ts.isNumericLiteral(name)) {
		return name.text;
	}

	if (ts.isComputedPropertyName(name)) {
		const expression = unwrapExpression(name.expression);
		if (ts.isStringLiteral(expression) || ts.isNoSubstitutionTemplateLiteral(expression)) {
			return expression.text;
		}
		throwUnsupportedTriggers(filePath, 'computed property names must be static');
	}

	return undefined;
}

function throwUnsupportedTriggers(filePath: string, reason: string): never {
	throw new Error(
		`[zene] Unsupported triggers export in ${filePath}: ${reason}. ` +
			'Use a static object literal, for example: export const triggers = { webhook: true }.',
	);
}
